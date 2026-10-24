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
- **Now-playing details**: a larger, read-only view of the current track
  (art and metadata). Playback controls stay in the bottom bar.

The panel slot itself (region 03), its toggle, width persistence, startup
precedence, and screen selection were implemented in `ui_01.md` phases 1–2.
This plan covers the screens, their data flow, and the engine work needed to
feed them. The now-playing layout is settled in §8.1; the queue layout is
still open (§8.2).

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
- **The now-playing panel is details-only.** No transport, seek, or volume
  controls; those live in the bar. Its state needs no seek draft and no
  volume memory.

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
- Neither panel screen has intents yet (`Emit` is empty). When row actions or
  other controls land, a helper maps each screen's `Emit` to the existing
  `Msg::Engine(...)` variants.

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
}

pub enum Msg {
    Sync(Arc<PlayerSnapshot>),
    CoverLoaded(CoverId, Result<Bytes, ()>),
}

pub enum Event {} // details-only: no transport/seek/volume intents

pub struct Init {
    pub manager: Arc<Manager>,
    pub snapshot: Arc<PlayerSnapshot>,
}
```

- On `Sync`, store the snapshot. If `now_playing.track.cover` differs from
  `cover_id`, clear the handle and spawn a fetch task. Ignore `CoverLoaded`
  for a stale id.
- **Fetch size**: request 2048 px from `Manager::cover`. Offline providers
  ignore the px and return the stored art; network providers may honour it.
  iced downscales for display.
- **Display size** is 80% of the panel body's width, square, computed with
  `responsive` — `FillPortion` cannot set a child's width. If the panel is
  wide relative to its height the cover can crowd the metadata; cap it by
  height if that shows up in practice.
- **No shared widgets with the bar.** The bar's compact cover/transport/
  progress/volume rows stay in `MainScreen`; the panel is a distinct
  full-size view.
- **No seek draft.** The bar remains the only seek control. The earlier
  panel draft's `SeekDragged`/`SeekReleased` messages and `Event` variants
  are removed.
- The bar keeps its own `cover`, `CoverLoaded`, `seeking`, and fetch path in
  `MainScreen`; it is chrome, not a screen.

### Cover ownership and duplicate fetches

The bar fetches ~96 px for a 48 px display. The panel fetches 2048 px for a
large display. The same track is therefore fetched twice at different sizes;
iced caches the handles independently. This is accepted: a shared cache would
move handles back into the coordinator and break the ownership rule.

Open question: gate the panel fetch on visibility (only fetch when the panel
shows Now-playing) or fetch on every track change even while hidden. Default
proposal: fetch always, matching the bar; add gating only if the 2048 px
texture while hidden matters.

## 7. Retention

- **Data**: retained because both screen states live in `MainScreen`, are kept
  synced while hidden, and are never recreated by toggling.
- **Cover handle**: retained with `now_playing_panel::State`.
- **Scroll position**: still relevant for the queue list once its layout is
  decided. Closing the panel currently calls `pane_grid::State::close`, which
  drops the panel's widget `Tree`. Two options:
  1. keep the panel pane mounted at zero width and treat `panel_open` as a
     ratio threshold (retains trees naturally);
  2. keep the current close/reopen and mirror scroll offsets in screen state
     via `scrollable::Id` + `snap_to` on activation.
  The plan assumes (1) unless it fights `pane_grid`; validate before deciding.

## 8. Visual design

### 8.1 Now-playing (decided)

```
┌────────────────────────────────────┐
│ [Queue] [Now playing]       [Hide] │
├────────────────────────────────────┤
│                                    │
│      ┌──────────────────────┐      │
│      │                      │      │
│      │                      │      │
│      │        COVER         │      │  width = 80% of body,
│      │                      │      │  height = same (square)
│      │                      │      │
│      └──────────────────────┘      │
│                                    │
│            Track title             │
│            Album                   │
│            Artist                  │
│            04:12                   │
│                                    │
│                                    │
│         (Space::new())             │  ← future visualizer
│                                    │
└────────────────────────────────────┘
```

- Everything is horizontally centered.
- Metadata order is **title, album, artist, duration**. Duration reuses
  `fmt_duration` (`04:12`).
- **No year.** `library::Track` does not carry one, and adding it is deferred.
  The album line is just the album name, with no dangling separator.
- No transport, progress, or volume controls; those stay in the bottom bar.
- Below the metadata, add `Space::new()` (Shrink × Shrink) with a
  `// future: visualizer` comment. It reserves no space now and is the seam
  for a future visualizer (algorithm out of scope).
- **Cover**: square, 80% of the panel body width, computed with `responsive`.
  Fetch at 2048 px and let iced downscale.
- **Empty state**: replace the cover with `lucide::music()` at
  `size(cover_side)` in the muted color, centered, with
  `"No music currently playing"` below it. Omit the metadata lines.
- **Loading placeholder**: the same muted icon at a smaller size, as in the
  bar.
- The panel header (tabs + Hide) stays as implemented.

### 8.2 Queue (undecided)

Do not implement the queue layout until these are settled. The architecture
(§2–§7) can land first with the current placeholder still rendering.

Open questions:

- **Queue rows**: title only, title + artist, duration, track number? Covers
  per row (implies per-row handles and fetches) or text-only?
- **Queue header**: "Up next" only, or also show the current track at the top
  highlighted as "Now playing"?
- **Queue length**: show upcoming only (engine queue) — confirmed for this
  plan — but should the current track be pinned above it?
- **Empty state**: what the queue shows when there is nothing upcoming.
- **Scroll**: the retention approach in §7 depends on whether the list uses
  `scrollable`.

## 9. Implementation order

1. **Engine**: `QueueChanged(Arc<[Track]>)`, emission per §3.2, fix mock
   `PlayNext`. Add tests asserting the emitted queue after each command.
2. **Coordinator plumbing**: `PlayerSnapshot`, `MainScreen` fields, rebuild +
   fan-out, both screen states created and synced; views stay placeholders.
3. **Queue screen**: `Screen` impl + read-only view using `snapshot.queue`.
4. **Now-playing screen**: state/messages, cover fetch, centered details view
   per §8.1, empty/loading states. No transport/seek/volume wiring.
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
  - Now-playing shows the current cover, title, album, artist, and duration,
    centered; no year, transport, progress, or volume controls;
  - changing tracks updates the panel cover; stale cover results are ignored;
  - with nothing playing, the panel shows the scaled muted icon and
    "No music currently playing";
  - switching Queue ↔ Now-playing and toggling the panel retains each
    screen's data (and scroll once §7 is decided);
  - panel startup/toggle persistence from `ui_01.md` still works.

## 11. Non-goals / future

- Queue row actions (remove, reorder, play-from-queue). Requires new engine
  commands; the screen's `Event` is the seam.
- Playback history view.
- Lyrics, visualizer (the `Space::new()` seam is reserved), equalizer.
- Track year metadata (deferred; the now-playing panel omits year).
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
- Covers and scroll are screen-owned. The bar's seek draft stays in
  `MainScreen`; the details-only panel has none.
