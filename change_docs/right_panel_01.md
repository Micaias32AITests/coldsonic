# Right panel 01 — Queue and Now-playing screens

Status: planned. Companion to `ui_01.md` (phase 4), which reserves region 03 but
deliberately leaves its contents underspecified. This doc supersedes
`ui_01.md` §5 where the two disagree.

Applies to: `src/engine/mod.rs`, `src/engine/queue.rs` (new),
`src/engine/mock.rs`, `src/engine/real.rs`, `src/screen/mod.rs`,
`src/screen/queue.rs`, `src/screen/now_playing_panel.rs`.

## Goal

Fill the right panel slot with two real screens:

- **Queue**: the current track pinned above a read-only view of the engine's
  upcoming tracks.
- **Now-playing details**: a larger, read-only view of the current track (art
  and metadata). Playback controls stay in the bottom bar.

The panel slot itself (region 03), its toggle, width persistence, startup
precedence, and screen selection were implemented in `ui_01.md` phases 1–2.
This plan covers the screens, their data flow, the shared engine queue, and the
engine work needed to feed them. Both layouts are settled in §8.

## 1. Locked-in decisions

- **The engine stays the source of truth.** The queue and `NowPlaying` live in
  the engine. `MainScreen` owns the engine; panel screens read it through a
  read-only `Ctx` and never own playback state.
- **Shared state is borrowed, not broadcast.** `MainScreen` builds a per-call
  `Ctx<'a>` with references to `manager`, `engine`, and `now_playing`. Panel
  screens never store a snapshot handle, and there is no `Arc<PlayerSnapshot>`
  fan-out.
- **The queue is a dedicated type.** `src/engine/queue.rs` owns upcoming,
  history, current, and queue-entry identity. `MockEngine` and `RealEngine`
  embed the same `Queue` instead of each maintaining three parallel fields.
- **Queue entries have stable ids.** `QueueEntryId(u64)` is monotonic per
  `Queue`; it is not a UUID and not a raw index. Duplicate songs therefore get
  distinct rows.
- **Screens own presentation resources, not player state.** Cover handles
  belong to the screen that displays them. Seek drafts belong to the control
  being dragged, not to `Ctx`.
- **Screens are plain structs with inherent methods.** There is no `Screen`
  trait and no `ViewCtx`/`UpdateCtx` associated types. Read-only display data
  arrives through `Ctx`; user intents leave through `Event`/`Action::Emit`, as
  with every other screen.
- **Both panel screens are long-lived.** `queue::State` and
  `now_playing_panel::State` are created at startup, retain their local state
  while hidden, and are read through `Ctx` when shown. Toggling the panel does
  not recreate either screen.
- **Queue is read-only for now.** No remove/reorder/play-row actions in this
  plan; adding them later means new `EngineMsg`s and queue `Event` variants,
  not a rewrite.
- **History stays out of the UI for now.** The queue screen shows the current
  track plus upcoming tracks. Previous songs remain engine-only until the
  future queue/history work.
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
  Read it through a borrowed accessor:

  ```rust
  impl Engine {
      pub fn queue(
          &self,
      ) -> impl ExactSizeIterator<Item = &QueueEntry> + Clone + '_;
  }
  ```

- `QueueEntry` is `{ id: QueueEntryId, track: Track }`. The view reads
  `entry.track`; the id is available for future row actions.
- `now_playing` is the coordinator's cached engine snapshot, borrowed for the
  call.
- No cover handles, no seek draft, no scroll position, no `last_volume`, and no
  `Arc<PlayerSnapshot>` in `Ctx`.

`MainScreen` builds `Ctx` immediately before calling a panel screen in `update`
or `view`. The panel screens keep their own local UI state (cover handles,
visible range) but never cache queue or now-playing data. Hidden screens read
the same live context when they are next shown, so there is nothing to sync.

## 3. Engine changes

### 3.1 Remove `QueueChanged`

`EngineEvent::QueueChanged` is removed. With live `Ctx`, it is redundant:

- iced calls `view` after every handled message.
- Every queue mutation happens inside `MainScreen::update` (a UI command routed
  to `engine.update`) or `Msg::Tick` (which calls `engine.tick`).
- The next `view` therefore reads the already-updated queue through
  `ctx.engine.queue()`.

The queue screen's only side effect is cover prefetching, which is driven
locally by `queue::Msg::Refresh` on `Tick`/panel open; it does not need an
engine event. This supersedes the `QueueChanged` mentions in `engine_01.md` and
`now_playing_02.md`.

### 3.2 Introduce a shared `Queue`

Add `src/engine/queue.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueueEntryId(u64);

#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub id: QueueEntryId,
    pub track: Track,
}

#[derive(Debug, Default)]
pub struct Queue {
    upcoming: VecDeque<QueueEntry>,
    history: VecDeque<QueueEntry>,
    current: Option<QueueEntry>,
    next_id: u64,
}
```

- `Queue` owns ordering, the current track, history, and entry ids. The engines
  own audio, position, volume, and playback state.
- `MockEngine` and `RealEngine` embed `queue: Queue` and stop maintaining their
  own `queue`/`history`/`current` fields.
- Queue methods cover the existing behaviour: replace/play queue, play now,
  enqueue, next/advance, previous/restart, stop, current access, and borrowed
  iteration over upcoming.
- `Stop` clears current and history but keeps upcoming, matching today's
  behaviour.
- `PlayQueue` clears history, replaces upcoming, and makes the first track
  current.
- History is retained for `Prev` but is not exposed to the panel.

`QueueEntryId` is a monotonic `u64` allocated by `Queue`:

- 8 bytes per entry; a 5k-track queue costs 40 KB of ids.
- IDs are never reused, so remove/reorder cannot silently retarget a row.
- Linear search by id over 5k entries is trivial; no secondary index is needed
  unless profiling later says otherwise.
- The counter is not reset by `PlayQueue`; it is only unique within the
  process lifetime.

This is the point where the queue becomes extensible. Future operations such as
remove, move, clear, and play-from-queue become `Queue` methods plus `EngineMsg`
variants; they do not require replacing the data structure.

### 3.3 Command semantics

Rename `EngineMsg::PlayNext(Track)` to `PlayNow(Track)`, because that is what
it actually does. Keep `EngineMsg::Enqueue(Track)` as append-to-end.

| Command / path           | Queue effect                                       |
| ------------------------ | -------------------------------------------------- |
| `PlayQueue`              | replace upcoming, clear history, first becomes current |
| `PlayNow`                | current moves to history, new track becomes current; upcoming unchanged |
| `Enqueue`                | append to the end of upcoming                       |
| `Next`                   | current moves to history, front of upcoming becomes current |
| auto-advance             | same as `Next`                                      |
| `Prev` (restart current) | current/upcoming unchanged; position restarts       |
| `Prev` (history step)    | current moves to front of upcoming, history tail becomes current |
| `Stop`                   | current/history clear; upcoming retained            |

`PlayNext` is reserved for a future “insert after current without interrupting”
command. It should not be used for immediate playback. This supersedes the
`PlayNext` naming in `engine_01.md`.

The mock currently pushes the new track onto the front of the queue *and* makes
it current. With `PlayNow`, the mock must not touch upcoming at all.

### 3.4 Borrowed queue accessor

```rust
impl Engine {
    pub fn queue(
        &self,
    ) -> impl ExactSizeIterator<Item = &QueueEntry> + Clone + '_;
}
```

The accessor is implemented by both engine variants and returns an iterator over
the same upcoming storage the player mutates. `Clone + ExactSizeIterator` lets
`lazy_list(ctx.engine.queue(), 56.0, ...)` consume it directly.

There is no mutable queue accessor. Mutations go upward as `EngineMsg`; screens
never borrow `&mut Engine`.

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
- `Msg::Tick`: after `engine.tick()` and updating `self.now_playing`, if the
  panel is open and showing Queue, call `queue::update(state, ctx,
  Msg::Refresh)` and collect its task. If it is showing Now playing, call
  `now_playing_panel::update(state, ctx, Msg::Refresh)`.
- On `TogglePanel` (opening) and on `SwitchPanel`, dispatch an immediate
  `Msg::Refresh` to the selected panel screen.
- `view_panel`: match `panel_screen` and call the corresponding inherent
  `view(state, ctx)`. The panel header (tabs, Hide) stays in `MainScreen` for
  now.
- Neither panel screen has intents yet (`Event` is empty). When row actions or
  other controls land, a helper maps each screen's `Event` to the existing
  `Msg::Engine(...)` variants.

No `PlayerSnapshot` type or snapshot field is introduced. The only shared state
is the queue/now-playing state already owned by `MainScreen`. No
`EngineEvent::QueueChanged` match arm exists.

## 5. Queue screen (`src/screen/queue.rs`)

```rust
pub struct State {
    covers: HashMap<CoverId, CoverState>,
    /// Upcoming entries currently visible to lazy-list cover prefetch.
    visible: Range<usize>,
}

enum CoverState {
    Loading,
    Loaded(image::Handle),
    Failed,
}

pub enum Msg {
    /// Re-scan current + visible entries for missing covers.
    Refresh,
    CoverLoaded(CoverId, Result<Bytes, ()>),
    /// Visible range changed; fetch newly visible covers.
    Scrolled(scrollable::Viewport),
}

pub enum Event {} // future: PlayTrack(QueueEntryId), Remove(QueueEntryId), ...

pub fn init() -> State;
pub fn update(
    state: &mut State,
    ctx: Ctx<'_>,
    msg: Msg,
) -> Action<Msg, Event>;
pub fn view<'a>(state: &'a State, ctx: Ctx<'a>) -> Element<'a, Msg>;
```

- `State` holds only local presentation state: cover handles and the visible
  range. It never stores a queue copy or the current track.
- `init` seeds `visible` with the first screenful (for example `0..24`) so the
  first covers can be requested before the first scroll event. All ranges are
  clamped to the current queue length.
- `Refresh` reads `ctx.now_playing` and the visible slice of
  `ctx.engine.queue()`, deduplicates by `CoverId`, and spawns fetches for ids
  that are not already `Loading`/`Loaded`. `Arc::clone(ctx.manager)` is used as
  the async task handle.
- `Scrolled` updates `visible` from the viewport and refreshes covers. Covers
  outside the current window (plus a small overscan) may be pruned so scrolling
  through a 5k-track queue does not retain every texture.
- `CoverLoaded` replaces the matching entry; results for pruned/unknown ids are
  ignored.
- Cover requests use 96 px, matching the bar's 48 px display and the other
  tables.
- `view`:
  - Pinned current row from `ctx.now_playing`, with a lighter surface than the
    panel background.
  - `UP NEXT` header with the queue length.
  - Upcoming rows via `lazy_list(ctx.engine.queue(), ROW_HEIGHT, ...)`.
  - Each row: 48 px cover (or muted `lucide::music` placeholder), title,
    artist, right-aligned duration.
  - Fixed `ROW_HEIGHT = 56.0`.
  - Long titles use no wrapping and are clipped; they must not change row
    height.
  - Empty states:
    - current exists, queue empty → pinned current + “Nothing up next”
    - nothing playing, queue empty → icon + “Nothing playing”
    - stopped but queue non-empty → show the list, with no pinned current
- Read-only: no row buttons and no queue mutations in this plan.
- No history is rendered. `Event` stays empty until row actions land.

The current row is pinned, not part of the scrollable list. That keeps it
visible and avoids having to locate it inside `lazy_list`.

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
  progress/volume rows stay in `MainScreen`; the panel is a distinct
  full-size view.
- **No seek draft.** The bar remains the only seek control.
- The bar keeps its own `cover`, `CoverLoaded`, `seeking`, and fetch path in
  `MainScreen`; it is chrome, not a screen.

### Cover ownership and duplicate fetches

The bar fetches ~96 px for a 48 px display. The panel fetches 2048 px for a
large display. The same track is therefore fetched twice at different sizes;
iced caches the handles independently. This is accepted: a shared cache would
move handles back into the coordinator and break the ownership rule.

The panel fetches only while the Now-playing tab is visible: `MainScreen`
dispatches `Refresh` on every `Tick` while that tab is shown, and once when the
tab/panel is opened. A track change that happens while hidden is picked up on
the next open.

## 7. Retention

- **Data**: shared player data is not copied into the panel; both screen states
  live in `MainScreen` and read live state through `Ctx` when shown. They are
  never recreated by toggling.
- **Cover handles**: retained with each screen's `State`. The queue cache is
  bounded to the visible window; the now-playing cover is retained for the
  current track.
- **Scroll position**: relevant for the queue list. Closing the panel currently
  calls `pane_grid::State::close`, which drops the panel's widget `Tree`;
  switching Queue ↔ Now-playing also replaces the body. Two options:
  1. keep both panel bodies mounted and toggle visibility (retains trees
     naturally);
  2. mirror the scroll offset in `queue::State` and restore it when shown.
  The plan assumes (1) unless it fights `pane_grid`; validate before deciding.
- The pinned current row does not scroll, so it needs no retention handling.

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
  “No music currently playing” below it. Omit the metadata lines.
- **Loading placeholder**: the same muted icon at a smaller size, as in the bar.
- The panel header (tabs + Hide) stays as implemented.

### 8.2 Queue (decided)

```
┌──────────────────────────────────────┐
│ [Queue] [Now playing]         [Hide] │
├──────────────────────────────────────┤
│ NOW PLAYING                          │
│ ┌────┐ Track title          03:45    │  ← lighter surface
│ │art │ Artist                       │
│ └────┘                               │
│ ──────────────────────────────────── │
│ UP NEXT                          12  │
│ ┌────┐ Track title          04:12    │
│ │art │ Artist                       │
│ └────┘                               │
│ ┌────┐ Track title          02:58    │
│ │art │ Artist                       │
│ └────┘                               │
│                  ⋮                   │
└──────────────────────────────────────┘
```

- The current track is pinned above the list and has a slightly lighter
  background; it is not part of the scrollable queue.
- Each upcoming row is 56 px tall, with a 48 px cover, title, artist, and
  right-aligned duration.
- The header shows `UP NEXT` and the number of upcoming tracks.
- No history/previous songs, no row actions, no dividers.
- Missing covers use the muted music icon; no per-row loading text.
- The current row is omitted when nothing is playing.
- Empty states are described in §5.

## 9. Implementation order

1. **Engine queue**: add `src/engine/queue.rs` with `Queue`, `QueueEntry`, and
   `QueueEntryId`; move mock/real queue state onto it; rename `PlayNext` to
   `PlayNow`; keep `Enqueue`; fix the mock; add the borrowed `Engine::queue()`
   accessor; remove `EngineEvent::QueueChanged`. Add queue tests.
2. **Coordinator plumbing**: `MainScreen` fields, `Ctx` construction in
   `update`/`view`, both panel states created, Tick/panel-open `Refresh`
   dispatch; views stay placeholders until step 3–4.
3. **Queue screen**: state/messages, visible-window cover fetch, `lazy_list`
   rows, pinned current row, empty states per §8.2.
4. **Now-playing screen**: state/messages, `Refresh` handling, cover fetch,
   centered details view per §8.1, empty/loading states. No
   transport/seek/volume wiring.
5. **Retention**: validate the keep-bodies-mounted approach; otherwise mirror
   the queue scroll offset.
6. **Visibility gating** is part of step 4: fetch the now-playing cover only
   while its tab is visible, and refresh on tab/panel open.

## 10. Verification

- `cargo build`, `cargo clippy --all-targets`, `cargo test` clean.
- Queue tests:
  - `PlayQueue` replaces upcoming, clears history, and makes the first track
    current.
  - `Enqueue` appends without changing current.
  - `PlayNow` changes current and history but not upcoming.
  - `Next` and auto-advance pop the front and push the old current to history.
  - History `Prev` pushes current back onto the queue front; restart `Prev`
    changes neither.
  - `Stop` clears current/history but keeps upcoming.
  - IDs stay unique for duplicate tracks and survive reorder.
  - Mock and real engines produce the same queue transitions.
- Manual:
  - play an artist → open Queue → current row is pinned and upcoming tracks
    match playback order;
  - 48 px covers appear, duplicate covers are fetched once, and scrolling
    fetches newly visible covers;
  - Next/Prev/auto-advance update the pinned row and list;
  - no previous songs appear;
  - long titles do not change row height;
  - Now-playing shows the current cover, title, album, artist, and duration,
    centered; no year, transport, progress, or volume controls;
  - changing tracks updates the panel cover; stale cover results are ignored;
  - with nothing playing, the panel shows the scaled muted icon and
    “No music currently playing”;
  - switching Queue ↔ Now-playing and toggling the panel retains each
    screen's data (and scroll once §7 is decided);
  - panel startup/toggle persistence from `ui_01.md` still works.

## 11. Non-goals / future

- Queue row actions (remove, reorder, play-from-queue, clear). Requires new
  `EngineMsg` variants and row events carrying `QueueEntryId`; `Queue` is ready
  for them.
- Previous-songs/history UI and a unified history/current/upcoming list.
- Auto-centering the current row when opening the panel or changing tracks.
  Deferred until the current track is inside the scrollable list. Requires a
  `LazyList` specialization (`center_on(index)` or equivalent) using the fixed
  56 px row height.
- Cover-tinted panel/list background derived from the current artwork.
- Lyrics, visualizer (the `Space::new()` seam is reserved), equalizer.
- Track year metadata (deferred; the now-playing panel omits year).
- Drag-and-drop inside the panel (`ui_01.md` non-goals).

## 12. Deviations from `ui_01.md` §5

- The panel is **not** a one-entry `Stack`. Both screens are long-lived,
  coordinator-owned states that read shared data through `Ctx`; this retains
  local cover/scroll state across toggles and screen switches without
  widget-tree tricks.
- There is no `QueueChanged` engine event. The queue is live through `Ctx`, and
  queue-screen cover refresh is driven by `Tick`/panel open.
- The queue is a shared `Queue` type with stable `QueueEntryId`s, not three
  separate fields duplicated in each engine.
- There is no `ViewCtx`/`UpdateCtx` associated type and no `Screen` trait; the
  read-only `Ctx` is passed explicitly to inherent methods.
- Covers and scroll are screen-owned. The bar's seek draft stays in
  `MainScreen`; the details-only panel has none.
