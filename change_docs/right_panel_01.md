# Right panel 01 — Queue and Now-playing screens

Status: planned. Companion to `ui_01.md` (phase 4), which reserves region 03
but deliberately leaves its contents underspecified. This doc supersedes
`ui_01.md` §5 where the two disagree.

Applies to: `src/engine/mod.rs`, `src/engine/mock.rs`, `src/engine/real.rs`,
`src/screen/mod.rs`, `src/screen/queue.rs`,
`src/screen/now_playing_panel.rs`.

## Goal

Fill the right panel slot with two real screens:

- **Queue**: a read-only view of the engine's upcoming tracks.
- **Now-playing details**: a larger player view for the current track,
  reusing the bottom bar's cover/transport/progress/volume pieces.

The panel slot itself (region 03), its toggle, width persistence, startup
precedence, and screen selection were implemented in `ui_01.md` phases 1–2.
This plan covers the screens, their data flow, and the engine work needed to
feed them. **Their visual design is not decided yet** — see §8.

## 1. Locked-in decisions

- **The engine stays the source of truth.** `queue` and `NowPlaying` live in
  the engine. The UI never borrows the engine from a screen and never owns
  playback state.
- **The coordinator owns one immutable UI snapshot.** `MainScreen` builds
  `PlayerSnapshot` from engine data and broadcasts it to screens as an
  `Arc`. Screens own their snapshot handle and read it in `view`.
- **Screens own presentation resources, not player state.** Cover handles
  belong to the screen that displays them; they are not part of the snapshot.
  Seek drafts belong to the control being dragged, not to the shared snapshot.
- **`Screen` is unchanged.** No `ViewCtx`/`UpdateCtx` associated types.
  Read-only display data arrives through `Msg`; user intents leave through
  `Emit`, as with every other screen. The idea of a view context was
  considered and rejected because ownership removes the borrowing problem.
- **Both panel screens are long-lived.** `queue::State` and
  `now_playing_panel::State` are created at startup, kept in sync even while
  hidden, and only their visibility changes. Toggling the panel does not
  recreate either screen.
- **Queue is read-only for now.** No remove/reorder/play-row actions in this
  plan; adding them later means new `EngineMsg`s and queue `Emit` variants,
  not a rewrite.

## 2. Snapshot model

```rust
/// Engine-derived data, rebuilt by `MainScreen` and pushed to screens.
pub struct PlayerSnapshot {
    pub now_playing: Option<Arc<NowPlaying>>,
    pub queue: Arc<[Track]>,
}
```

- `queue` is the upcoming tracks (the engine's queue), not current/history.
- `now_playing` is the existing engine snapshot, wrapped once in an `Arc` per
  rebuild.
- No cover handles, no seek draft, no scroll position, no `last_volume`.

`MainScreen` keeps `snapshot: Arc<PlayerSnapshot>` and rebuilds it after every
engine event and on every tick (position changes every tick). The rebuild
allocates one `Arc`; the `now_playing` and queue payloads are shared.

Broadcasting:

```rust
// after rebuilding the snapshot
let snapshot = Arc::clone(&self.snapshot);
let queue_task = self.queue_screen.update(queue::Msg::Sync(snapshot.clone()));
let panel_task = self.now_playing_screen.update(now_playing_panel::Msg::Sync(snapshot));
```

Both updates are run unconditionally; the screens are cheap and hidden
screens must not go stale.

## 3. Engine changes

### 3.1 Queue events carry a snapshot

```rust
// src/engine/mod.rs
EngineEvent::QueueChanged(Arc<[Track]>),
```

`Arc<[Track]>` is cheap to clone through the coordinator into both panel
screens. The engine builds a fresh snapshot only when the queue actually
changes.

### 3.2 Emission points

Emit after the mutation, so the snapshot reflects the remaining queue:

| Command / path | Queue changes | Emit |
|---|---|---|
| `PlayQueue` | replaced, then first track popped as current | yes, **after** the pop |
| `PlayNext` | no (current changes, queue unchanged) | no |
| `Enqueue` | appended | yes |
| `Next` | front popped as current | yes |
| auto-advance | front popped as current | yes |
| `Prev` (history step) | current pushed to the front | yes |
| `Prev` (restart current) | no | no |
| `Stop` | no (current/history cleared, queue kept) | no |

`QueueChanged` is currently only emitted by `PlayQueue`, `PlayNext`, and
`Enqueue`, and `PlayNext` emits it although nothing changed. The table above
replaces that behaviour.

### 3.3 Fix `MockEngine::PlayNext`

The mock currently pushes the new track onto the **front of the queue** *and*
makes it current, so `Next` replays it. `RealEngine` only makes it current.
Fix the mock to match the real engine (no queue push) so the queue snapshots
agree across engines.

### 3.4 No queue accessor needed

Screens receive the queue through `PlayerSnapshot`; `MainScreen` never needs
to call into the engine during `view`. This keeps `view` free of engine
borrows.

## 4. Coordinator changes (`src/screen/mod.rs`)

Fields:

```rust
snapshot: Arc<PlayerSnapshot>,
queue_screen: queue::State,
now_playing_screen: now_playing_panel::State,
panel_screen: PanelScreen,   // already exists
panel_open: bool,            // already exists
seeking: Option<f32>,        // bar's own draft, already exists
```

- `init`: build an empty snapshot and both screen states, then send each its
  initial `Sync`. `queue::State` needs no other init data;
  `now_playing_panel::State` also receives `Arc<Manager>` for cover fetches.
- `apply_engine_event` / `Tick`: after updating `self.now_playing`, rebuild
  `self.snapshot` and fan it out to both screens, collecting their tasks.
- `Msg::EngineEvent`/`Tick` currently return tasks; keep that shape for the
  screens' sync tasks.
- `view_panel`: match `panel_screen` and call the corresponding `Screen::view`.
  The panel header (tabs, Hide) stays in `MainScreen` for now.
- Panel intents are handled by a helper that maps each screen's `Emit` to the
  existing `Msg::Engine(...)` variants.

`PlayerSnapshot` lives in `src/screen/mod.rs` initially; move it to its own
module only if it grows.

## 5. Queue screen (`src/screen/queue.rs`)

```rust
pub struct State { snapshot: Arc<PlayerSnapshot> }

pub enum Msg { Sync(Arc<PlayerSnapshot>) }

pub enum Event {} // future: PlayTrack(TrackId), Remove(usize), ...

impl Screen for State {
    type Msg = Msg;
    type Emit = Event;
    type InitData = Arc<PlayerSnapshot>;
    // view reads self.snapshot.queue
}
```

- `update(Msg::Sync)` replaces the snapshot.
- Reads `self.snapshot.queue`; rows borrow `&Track`, so no per-frame string
  clones.
- Empty state when the queue is empty.
- Read-only: no row buttons in this plan.
- Long queues rebuild every 250 ms tick. Text rows are fine for normal
  queues; a pathological queue is LazyList territory (`ui_01.md` §7). If that
  becomes the blocker, stop and raise it rather than building LazyList here.

Visual structure is not decided (§8).

## 6. Now-playing screen (`src/screen/now_playing_panel.rs`)

```rust
pub struct State {
    manager: Arc<Manager>,
    snapshot: Arc<PlayerSnapshot>,
    cover: Option<image::Handle>,
    /// Cover id `cover` belongs to, for stale-result filtering.
    cover_id: Option<CoverId>,
    /// This control's own seek draft.
    seeking: Option<f32>,
}

pub enum Msg {
    Sync(Arc<PlayerSnapshot>),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    SeekDragged(f32),
    SeekReleased,
}

pub enum Event {
    Seek(f32),
    PlayPause,
    Next,
    Prev,
    Stop,
    SetVolume(f32),
}

pub struct Init {
    pub manager: Arc<Manager>,
    pub snapshot: Arc<PlayerSnapshot>,
}
```

- On `Sync`, store the snapshot. If `now_playing.track.cover` differs from
  `cover_id`, clear the handle and spawn a fetch task at the panel's own
  resolution. Ignore `CoverLoaded` for a stale id.
- `SeekDragged` stores the draft; `SeekReleased` takes it and emits
  `Event::Seek`. The engine remains the only writer of authoritative
  position; after `SeekDone`/the next tick the snapshot updates both sliders.
- Volume slider emits `Event::SetVolume` continuously (harmless). The mute
  toggle needs remembered non-zero volume; keep that memory in the panel (or
  coordinator) as a UI-only value — do not put it in `PlayerSnapshot`.
- Transport/progress/volume/cover **components are extracted** from
  `MainScreen::view_now_playing` into reusable view functions so the bar and
  the panel build the same rows. Their exact API (props struct, size/
  compactness flags) is decided with the visual design.
- The bar keeps its own `cover`, `CoverLoaded`, `seeking`, and fetch path in
  `MainScreen`; it is chrome, not a screen.

### Cover ownership and duplicate fetches

The bar fetches ~96 px for a 48 px display. The panel fetches its own handle
at its own resolution (value TBD). The same track may therefore be fetched
twice, at different sizes; iced caches the handles independently. This is
accepted: a shared cache would move handles back into the coordinator and
break the ownership rule.

Open question: gate the panel fetch on visibility (only fetch when the panel
shows Now-playing) or fetch on every track change even while hidden. Default
proposal: fetch always, matching the bar; add gating only if it matters.

## 7. Retention

- **Data**: retained because both screen states live in `MainScreen`, are kept
  synced while hidden, and are never recreated by toggling.
- **Cover handle**: retained with `now_playing_panel::State`.
- **Scroll position**: **not yet solved.** Closing the panel currently calls
  `pane_grid::State::close`, which drops the panel's widget `Tree`. Two options
  once the visual design exists:
  1. keep the panel pane mounted at zero width and treat `panel_open` as a
     ratio threshold (retains trees naturally);
  2. keep the current close/reopen and mirror scroll offsets in screen state
     via `scrollable::Id` + `snap_to` on activation.
  The plan assumes (1) unless it fights `pane_grid`; validate before deciding.

## 8. Visual design — deliberately undecided

Do not implement the layout until these are settled. The architectural work
(§2–§7) can land first with the current placeholders still rendering.

Open questions:

- **Panel width**: current default is 20% of the window; is that enough for a
  now-playing view?
- **Queue rows**: title only, title + artist, duration, track number? Covers
  per row (implies per-row handles and fetches) or text-only?
- **Queue header**: "Up next" only, or also show the current track at the top
  highlighted as "Now playing"?
- **Queue length**: show upcoming only (engine queue) — confirmed for this
  plan — but should the current track be pinned above it?
- **Now-playing layout**: large cover size and placement; title/artist/album
  lines; where transport, progress, and volume sit vertically.
- **Component API**: do the bar pieces take a `compact: bool`, explicit sizes,
  or separate compact/full builders?
- **Cover sizes**: display size and fetch size (2× for HiDPI) for the panel.
- **Empty states**: no track playing, empty queue.
- **Panel header**: keep tabs + Hide, or move the toggle to the bar only?

## 9. Implementation order

1. **Engine**: `QueueChanged(Arc<[Track]>)`, emission per §3.2, fix mock
   `PlayNext`. Add tests asserting the emitted queue after each command.
2. **Coordinator plumbing**: `PlayerSnapshot`, `MainScreen` fields, rebuild +
   fan-out, both screen states created and synced; views stay placeholders.
3. **Queue screen**: `Screen` impl + read-only view using `snapshot.queue`.
4. **Now-playing screen**: state/messages/emit, cover fetch, seek draft;
   extract shared components; view per §8 once decided.
5. **Retention**: validate the keep-pane-mounted approach; otherwise implement
   scroll mirroring.
6. **Visibility gating** for cover fetches (only if needed).

## 10. Verification

- `cargo build`, `cargo clippy --all-targets`, `cargo test` clean.
- Engine tests: queue snapshot excludes the current track after `PlayQueue`;
  `Next`, auto-advance, `Enqueue`, and history `Prev` update the snapshot;
  restart-current `Prev`, `PlayNext`, and `Stop` do not.
- Manual:
  - play an artist → open Queue → upcoming tracks match playback order;
  - Next/Prev/auto-advance update the queue within one tick;
  - Now-playing shows the current track, transport works, seek drags without
    the tick fighting the thumb, volume/mute work;
  - switching Queue ↔ Now-playing and toggling the panel retains each
    screen's data (and scroll once §7 is decided);
  - panel startup/toggle persistence from `ui_01.md` still works.

## 11. Non-goals / future

- Queue row actions (remove, reorder, play-from-queue). Requires new engine
  commands; the screen's `Event` is the seam.
- Playback history view.
- Lyrics, visualizer, equalizer.
- Drag-and-drop inside the panel (`ui_01.md` non-goals).
- LazyList windowing: same blockage rule as `ui_01.md` §7 — if a queue is too
  large to render without it, stop and raise it.

## 12. Deviations from `ui_01.md` §5

- The panel is **not** a one-entry `Stack`. Both screens are long-lived,
  coordinator-owned states fed by `Arc<PlayerSnapshot>`; this retains data
  across toggles and screen switches without widget-tree tricks.
- `EngineEvent::QueueChanged` gains an `Arc<[Track]>` payload and is emitted
  from every path that mutates the queue.
- `ViewCtx` on `Screen` was considered and rejected; `Screen` stays as it is.
- Covers, seek drafts, and scroll are screen/control-owned, never snapshot
  state.
