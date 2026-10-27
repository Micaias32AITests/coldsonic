# Right panel 01 — Queue and Now-playing screens

Status: planned. Companion to `ui_01.md` (phase 4), which reserves region 03 but
deliberately leaves its contents underspecified. This doc supersedes `ui_01.md`
§5 where the two disagree.

Applies to: `src/engine/mod.rs`, `src/engine/mock.rs`, `src/engine/real.rs`,
`src/screen/mod.rs`, `src/screen/queue.rs`, `src/screen/now_playing_panel.rs`.

## Goal

Fill the right panel slot with two real screens:

- **Queue**: a read-only (for now) view of the engine's upcoming tracks.
- **Now-playing details**: a larger, read-only view of the current track (art
  and metadata). Playback controls stay in the bottom bar.

The panel slot itself (region 03), its toggle, width persistence, startup
precedence, and screen selection were implemented in `ui_01.md` phases 1–2. This
plan covers the screens, their data flow, and the engine work needed to feed
them. The now-playing layout is settled in §8.1; the queue layout is still open
(§8.2).

## 1. Locked-in decisions

- **The engine stays the source of truth.** `queue` and `NowPlaying` live in the
  engine. `MainScreen` owns the engine; panel screens read it through a
  read-only `Ctx` and never own playback state.
- **Shared state is borrowed, not broadcast.** `MainScreen` builds a per-call
  `Ctx<'a>` with references to `manager`, `engine`, and `now_playing`. Panel
  screens never store a snapshot handle, and there is no `Arc<PlayerSnapshot>`
  fan-out.
- **Screens own presentation resources, not player state.** Cover handles belong
  to the screen that displays them. Seek drafts belong to the control being
  dragged, not to `Ctx`.
- **Screens are plain structs with inherent methods.** There is no `Screen`
  trait and no `ViewCtx`/`UpdateCtx` associated types. Read-only display data
  arrives through `Ctx`; user intents leave through `Event`/`Action::Emit`, as
  with every other screen.
- **Both panel screens are long-lived.** `queue::State` and
  `now_playing_panel::State` are created at startup, retain their local state
  while hidden, and are read through `Ctx` when shown. Toggling the panel does
  not recreate either screen.
- **Queue is read-only for now.** No remove/reorder/play-row actions in this
  plan; adding them later means new `EngineMsg`s and queue `Emit` variants, not
  a rewrite.
- **The now-playing panel is details-only.** No transport, seek, or volume
  controls; those live in the bar. Its state needs no seek draft and no volume
  memory.

## 2. Read-only context model

Panel screens read shared state through `MainScreen`'s `Ctx<'a>`:

```rust
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub manager: &'a Arc<Manager>,
    pub engine: &'a Engine,
    pub now_playing: Option<&'a NowPlaying>,
}
```

- The queue is the upcoming tracks (the engine's queue), not current/history.
  Read it through a borrowed engine accessor:

  ```rust
  impl Engine {
      pub fn queue(&self) -> impl ExactSizeIterator<Item = &Track> + '_;
  }
  ```

- `now_playing` is the coordinator's cached engine snapshot, borrowed for the
  call.
- No cover handles, no seek draft, no scroll position, no `last_volume`, and no
  `Arc<PlayerSnapshot>` in `Ctx`.

`MainScreen` builds `Ctx` immediately before calling a panel screen in `update`
or `view`. The panel screens keep their own local UI state (cover handles,
scroll position) but never cache queue or now-playing data. Hidden screens read
the same live context when they are next shown, so there is nothing to sync.

## 3. Engine changes

### 3.1 Queue events stay payload-free

```rust
// src/engine/mod.rs
EngineEvent::QueueChanged,
```

The engine keeps the queue; screens read it on demand through
`Ctx::engine.queue()`. The event only tells the coordinator that queue-derived
views may need repainting, so there is no `Arc<[Track]>` snapshot to build or
clone.

### 3.2 Emission points

Emit after the mutation, so the queue accessor reflects the remaining tracks:

| Command / path           | Queue changes                                | Emit                   |
| ------------------------ | -------------------------------------------- | ---------------------- |
| `PlayQueue`              | replaced, then first track popped as current | yes, **after** the pop |
| `PlayNext`               | no (current changes, queue unchanged)        | no                     |
| `Enqueue`                | appended                                     | yes                    |
| `Next`                   | front popped as current                      | yes                    |
| auto-advance             | front popped as current                      | yes                    |
| `Prev` (history step)    | current pushed to the front                  | yes                    |
| `Prev` (restart current) | no                                           | no                     |
| `Stop`                   | no (current/history cleared, queue kept)     | no                     |

`QueueChanged` is currently only emitted by `PlayQueue`, `PlayNext`, and
`Enqueue`, and `PlayNext` emits it although nothing changed. The table above
replaces that behaviour.

### 3.3 Fix `MockEngine::PlayNext`

The mock currently pushes the new track onto the **front of the queue** _and_
makes it current, so `Next` replays it. `RealEngine` only makes it current. Fix
the mock to match the real engine (no queue push) so the queue contents agree
across engines.

### 3.4 Borrowed queue accessor

Add a read-only accessor so panel views can borrow the queue without owning a
copy:

```rust
impl Engine {
    pub fn queue(&self) -> impl ExactSizeIterator<Item = &Track> + '_;
}
```

The accessor is implemented by both engine variants and returns an iterator over
the same queue storage the player mutates. A `view` that reads
`ctx.engine.queue()` holds a shared borrow of the engine only for the duration of
the call, and rows borrow `&Track` directly.

## 4. Coordinator changes (`src/screen/mod.rs`)

Fields:

```rust
queue_screen: queue::State,
now_playing_screen: now_playing_panel::State,
panel_screen: PanelScreen,   // already exists
panel_open: bool,            // already exists
seeking: Option<f32>,        // bar's own draft, already exists
```

- `init`: create both panel states with their inherent `init` functions. They
  start empty; there is no snapshot to seed.
- Build `Ctx` only while dispatching to a panel screen, from disjoint fields:
  `manager`, `engine`, and `now_playing`. Do not hold it across a mutation of
  `MainScreen`.
- `apply_engine_event` / `Tick`: after updating `self.now_playing`, call
  `now_playing_panel::update(state, ctx, Msg::Refresh)` so the panel can notice a
  track/cover change and spawn a fetch. The queue screen has no update messages
  in this plan; its view reads the live queue.
- `view_panel`: match `panel_screen` and call the corresponding inherent
  `view(state, ctx)`. The panel header (tabs, Hide) stays in `MainScreen` for
  now.
- Neither panel screen has intents yet (`Event` is empty). When row actions or
  other controls land, a helper maps each screen's `Event` to the existing
  `Msg::Engine(...)` variants.

No `PlayerSnapshot` type or snapshot field is introduced. The only shared state
is the engine/now-playing state already owned by `MainScreen`.

## 5. Queue screen (`src/screen/queue.rs`)

```rust
pub struct State { /* scroll state once the layout is decided */ }

pub enum Event {} // future: PlayTrack(TrackId), Remove(usize), ...

pub fn init() -> State;
pub fn view<'a>(state: &'a State, ctx: Ctx<'a>) -> Element<'a, Msg>;
```

- No `Msg::Sync`: the view reads `ctx.engine.queue()` directly, so there is no
  snapshot to replace.
- Rows borrow `&Track` from the engine, so there are no per-frame string clones.
- Empty state when the queue is empty.
- Read-only: no row buttons and no `update` in this plan. Add `Msg`/`Event` when
  row actions land.
- Long queues are rebuilt on each frame/tick that redraws the panel. Text rows
  are fine for normal queues; a pathological queue is LazyList territory
  (`ui_01.md` §7). If that becomes the blocker, stop and raise it rather than
  building LazyList here.

Visual structure is not decided (§8).

## 6. Now-playing screen (`src/screen/now_playing_panel.rs`)

```rust
pub struct State {
    cover: Option<image::Handle>,
    /// Cover id `cover` belongs to, for stale-result filtering.
    cover_id: Option<CoverId>,
}

pub enum Msg {
    /// Re-check the borrowed now-playing state for a track/cover change.
    Refresh,
    CoverLoaded(CoverId, Result<Bytes, ()>),
}

pub enum Event {} // details-only: no transport/seek/volume intents

pub fn init() -> State;
pub fn update(
    state: &mut State,
    ctx: Ctx<'_>,
    msg: Msg,
) -> Action<Msg, Event>;
pub fn view<'a>(state: &'a State, ctx: Ctx<'a>) -> Element<'a, Msg>;
```

- On `Refresh`, read `ctx.now_playing`. If the cover id differs from `cover_id`,
  clear the handle and spawn a fetch task using `Arc::clone(ctx.manager)` as the
  async task handle. Ignore `CoverLoaded` for a stale id.
- `view` reads `ctx.now_playing` for title/album/artist/duration and
  `state.cover` for the image.
- **Fetch size**: request 2048 px from `Manager::cover`. Offline providers
  ignore the px and return the stored art; network providers may honour it. iced
  downscales for display.
- **Display size** is 80% of the panel body's width, square, computed with
  `responsive` — `FillPortion` cannot set a child's width. If the panel is wide
  relative to its height the cover can crowd the metadata; cap it by height if
  that shows up in practice.
- **No shared widgets with the bar.** The bar's compact cover/transport/
  progress/volume rows stay in `MainScreen`; the panel is a distinct full-size
  view.
- **No seek draft.** The bar remains the only seek control. The earlier panel
  draft's `SeekDragged`/`SeekReleased` messages and `Event` variants are
  removed.
- The bar keeps its own `cover`, `CoverLoaded`, `seeking`, and fetch path in
  `MainScreen`; it is chrome, not a screen.

### Cover ownership and duplicate fetches

The bar fetches ~96 px for a 48 px display. The panel fetches 2048 px for a
large display. The same track is therefore fetched twice at different sizes;
iced caches the handles independently. This is accepted: a shared cache would
move handles back into the coordinator and break the ownership rule.

Open question: gate the panel fetch on visibility (only fetch when the panel
shows Now-playing) or fetch on every track change even while hidden. Default
proposal: fetch always, matching the bar; add gating only if the 2048 px texture
while hidden matters.

## 7. Retention

- **Data**: shared player data is not copied into the panel; both screen states
  live in `MainScreen` and read live state through `Ctx` when shown. They are
  never recreated by toggling.
- **Cover handle**: retained with `now_playing_panel::State`.
- **Scroll position**: still relevant for the queue list once its layout is
  decided. Closing the panel currently calls `pane_grid::State::close`, which
  drops the panel's widget `Tree`. Two options:
  1. keep the panel pane mounted at zero width and treat `panel_open` as a ratio
     threshold (retains trees naturally);
  2. keep the current close/reopen and mirror scroll offsets in screen state via
     `scrollable::Id` + `snap_to` on activation. The plan assumes (1) unless it
     fights `pane_grid`; validate before deciding.

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
  `// future: visualizer` comment. It reserves no space now and is the seam for
  a future visualizer (algorithm out of scope).
- **Cover**: square, 80% of the panel body width, computed with `responsive`.
  Fetch at 2048 px and let iced downscale.
- **Empty state**: replace the cover with `lucide::music()` at
  `size(cover_side)` in the muted color, centered, with
  `"No music currently playing"` below it. Omit the metadata lines.
- **Loading placeholder**: the same muted icon at a smaller size, as in the bar.
- The panel header (tabs + Hide) stays as implemented.

### 8.2 Queue (undecided)

Do not implement the queue layout until these are settled. The architecture
(§2–§7) can land first with the current placeholder still rendering.

Open questions:

- **Queue rows**: title only, title + artist, duration, track number? Covers per
  row (implies per-row handles and fetches) or text-only?
- **Queue header**: "Up next" only, or also show the current track at the top
  highlighted as "Now playing"?
- **Queue length**: show upcoming only (engine queue) — confirmed for this plan
  — but should the current track be pinned above it?
- **Empty state**: what the queue shows when there is nothing upcoming.
- **Scroll**: the retention approach in §7 depends on whether the list uses
  `scrollable`.

## 9. Implementation order

1. **Engine**: keep `QueueChanged` payload-free, emission per §3.2, add the
   borrowed `Engine::queue()` accessor, and fix mock `PlayNext`. Add tests
   asserting the queue contents after each command.
2. **Coordinator plumbing**: `MainScreen` fields, `Ctx` construction in
   `update`/`view`, both panel states created; views stay placeholders.
3. **Queue screen**: inherent `init`/`view` using `ctx.engine.queue()`.
4. **Now-playing screen**: state/messages, `Refresh` handling, cover fetch,
   centered details view per §8.1, empty/loading states. No
   transport/seek/volume wiring.
5. **Retention**: validate the keep-pane-mounted approach; otherwise implement
   scroll mirroring.
6. **Visibility gating** for cover fetches (only if needed).

## 10. Verification

- `cargo build`, `cargo clippy --all-targets`, `cargo test` clean.
- Engine tests: queue contents exclude the current track after `PlayQueue`;
  `Next`, auto-advance, `Enqueue`, and history `Prev` update the queue;
  restart-current `Prev`, `PlayNext`, and `Stop` do not.
- Manual:
  - play an artist → open Queue → upcoming tracks match playback order;
  - Next/Prev/auto-advance update the queue within one tick;
  - Now-playing shows the current cover, title, album, artist, and duration,
    centered; no year, transport, progress, or volume controls;
  - changing tracks updates the panel cover; stale cover results are ignored;
  - with nothing playing, the panel shows the scaled muted icon and "No music
    currently playing";
  - switching Queue ↔ Now-playing and toggling the panel retains each screen's
    data (and scroll once §7 is decided);
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
  coordinator-owned states that read shared data through `Ctx`; this retains
  local cover/scroll state across toggles and screen switches without
  widget-tree tricks.
- `EngineEvent::QueueChanged` remains payload-free and is emitted from every
  path that mutates the queue. Panel views read the queue through the borrowed
  `Engine::queue()` accessor.
- There is no `ViewCtx`/`UpdateCtx` associated type and no `Screen` trait; the
  read-only `Ctx` is passed explicitly to inherent methods.
- Covers and scroll are screen-owned. The bar's seek draft stays in
  `MainScreen`; the details-only panel has none.
