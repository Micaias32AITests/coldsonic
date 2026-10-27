# Screen 01 — Shared context and inherent screen methods

Status: implemented in code. The remaining `change_docs/` plans are aligned to
the inherent-method/`Ctx` convention. This doc supersedes the old
`right_panel_01.md` statements that `Screen` was unchanged and that it broadcast
a `PlayerSnapshot` as `Arc`. The panel layout and engine queue work in
`right_panel_01.md` remain valid; this doc changes only how screens receive
shared state and how their methods are organized.

Applies to: `src/lib.rs`, `src/prelude.rs`, `src/main.rs`, `src/screen/mod.rs`,
`src/screen/artists.rs`, `src/screen/library_setup/mod.rs`,
`src/screen/library_setup/list_libraries.rs`,
`src/screen/library_setup/add_offline.rs`, `src/screen/debug.rs`, and the
placeholder screen comments.

## Goal

Remove the associated context/`Screen` trait approach and replace it with two
explicit, ordinary Rust patterns:

1. **Named context struct** (`Ctx<'a>`) where several read-only values are
   shared down the hierarchy.
2. **Direct parameters** where a module only needs one or two values.

Shared state stays at the common ancestor (`MainScreen`) and is passed down by
reference. Child screens mutate through returned events (`Emit`), not by
receiving `&mut MainScreen`.

No `Arc` wrapping of live UI state, no cloning of live UI values, and no nightly
features.

## 1. Locked-in decisions

- **Drop the `Screen` trait.**
- **`MainScreen` owns the shared state.** It is the only owner of `manager`,
  `engine`, and `now_playing`.
- **`Ctx<'a>` is read-only.** It contains references, is `Copy`, and is built
  per call. It is never stored in screen state.
- **Mutations flow upward as events.** A child returns `Action::Emit(Event)`;
  `MainScreen` applies the mutation.
- **Direct parameters are allowed.** Setup/debug modules do not need `Ctx`; they
  take exactly the arguments they use.
- **`Arc::clone` only for async task handles.** A refcount bump to hand the
  manager to a spawned task is fine; cloning live UI values is not.
- **Subscriptions are screen-local API.** A screen that has a subscription
  exposes `pub(in crate::screen) fn subscription(...)`. If the parent forgets to
  call it, the normal `dead_code` warning flags it. Do not add
  `#[deny(dead_code)]`; a warning is enough.
- **No nightly.** A named struct or direct parameters need no GATs or associated
  type defaults.

## 2. Why drop the trait

The trait cannot cleanly carry a context for the whole hierarchy:

- `MainScreen` **owns** `manager` and `engine`; it cannot receive a `Ctx`
  borrowed from itself for its own `update`.
- `library_setup` and `debug` are root screens and do not have an engine at all.
- A single trait signature would force dummy contexts or a second trait.

There are only six `Screen` impls and no generic, `dyn`, or blanket use, so
removing the trait is mechanical. Each module gets ordinary inherent methods or
free functions.

## 3. Context model

Put this in `src/screen/mod.rs` near `MainScreen`:

```rust
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub manager: &'a Arc<Manager>,
    pub engine: &'a Engine,
    pub now_playing: Option<&'a engine::NowPlaying>,
    // add more fields only when a child screen actually reads them
}
```

Rules:

- `Ctx` contains **only shared read-only references** (plus `Copy` snapshots
  such as `Option<&NowPlaying>`).
- It is constructed per `update`/`view` call and never stored in a screen state.
- It must **not** contain `entries`, `cover`, `seeking`, `panes`, or any field
  the coordinator mutates while a child is borrowed.
- Add fields only when a screen needs them. Do not turn `Ctx` into a mirror of
  `MainScreen`.

## 4. Mutation flow

Child `update` returns `Action<Msg, Event>`.

`MainScreen` matches on the `Action` and handles `Event` directly:

```rust
let action = {
    // scoped borrows
    artists::update(state, ctx, msg)
};

match action {
    Action::None => Task::none(),
    Action::Task(task) => task.map(Msg::ArtistsScreenMsg),
    Action::Emit(event) => self.handle_artists_event(event),
}
```

Do not route child events through `Action::handle` when the handler needs
`&mut self`; direct matching avoids closure lifetime and manager-clone
workarounds.

## 5. Target APIs

### 5.1 `src/screen/artists.rs`

```rust
pub fn init(manager: Arc<Manager>) -> (State, Task<Msg>);

pub fn update(
    state: &mut State,
    ctx: Ctx<'_>,
    msg: Msg,
) -> Action<Msg, Event>;

pub fn view(state: &State) -> Element<'_, Msg>;
```

Changes:

- Remove `impl Screen`.
- Remove `manager: Arc<Manager>` from `State`.
- `update` uses `ctx.manager.clone()` only when spawning a cover-fetch task.
- `view` does not take `Ctx` yet because it reads no shared state. Add it later
  if the view needs engine/now-playing data.

### 5.2 `src/screen/mod.rs` — `MainScreen`

```rust
impl MainScreen {
    pub fn init(manager: Arc<Manager>) -> (Self, Task<Msg>);
    pub fn update(&mut self, msg: Msg) -> Task<Msg>;
    pub fn view(&self) -> Element<'_, Msg>;
    pub fn subscription(&self) -> Subscription<Msg>;
}
```

Changes:

- Remove `impl Screen for MainScreen`.
- Move the method bodies into the inherent `impl MainScreen`.
- `update` returns `Task<Msg>` instead of `Action<Msg, Never>`.
- `apply_engine_event` returns `Task<Msg>`.
- `handle_sidebar` returns `Task<Msg>`.
- Build `Ctx` only while dispatching to a child:

```rust
let action = {
    let MainScreen {
        entries,
        manager,
        engine,
        now_playing,
        ..
    } = self;

    let Some(entry) = entries.last_mut() else {
        return Task::none();
    };

    let ctx = Ctx {
        manager: &*manager,
        engine: &*engine,
        now_playing: now_playing.as_ref(),
    };

    match &mut entry.screen {
        SubScreen::Artists(state) => artists::update(state, ctx, msg),
        // ...
    }
};

match action { /* ... */ }
```

- Add `handle_artists_event(&mut self, event: artists::Event) -> Task<Msg>`.
- `Msg::ArtistsScreenMsg` should no longer clone `self.manager` just to capture
  it in an `Action::handle` closure.
- `make_screen` uses `artists::init(...).0` and maps the returned task.
- `view_route` calls `artists::view(state)`.

### 5.3 `src/screen/library_setup/mod.rs`

```rust
impl State {
    pub fn init() -> (Self, Task<Msg>);
    pub fn update(&mut self, msg: Msg) -> Action<Msg, Arc<Manager>>;
    pub fn view(&self) -> Element<'_, Msg>;
}
```

Remove `impl Screen`. Keep the event type as `Arc<Manager>`.

### 5.4 `src/screen/library_setup/list_libraries.rs`

```rust
impl State {
    // direct parameters: no tuple InitData
    pub fn init(libraries: Vec<LibraryConfig>, error: Option<String>) -> Self;
    pub fn update(&mut self, msg: Msg) -> Action<Msg, Event>;
    pub fn view(&self) -> Element<'_, Msg>;
}
```

### 5.5 `src/screen/library_setup/add_offline.rs`

```rust
impl State {
    pub fn init(libraries: Vec<LibraryConfig>) -> Self;
    pub fn update(&mut self, msg: Msg) -> Action<Msg, Event>;
    pub fn view(&self) -> Element<'_, Msg>;
}
```

### 5.6 `src/screen/debug.rs`

```rust
impl State {
    pub fn init() -> (Self, Task<Msg>);
    pub fn update(&mut self, msg: Msg) -> Action<Msg, Event>;
    pub fn view(&self) -> Element<'_, Msg>;
}
```

Remove the `Screen` import. `main.rs` can return `Subscription::none()` for the
debug variant instead of relying on the trait default.

### 5.7 `src/main.rs`

- Remove `use coldsonic::Screen`.
- Call inherent methods for `library_setup`, `debug`, and `MainScreen`.
- Remove `.to_task()` on `MainScreen::update`.
- For `State::subscription`, call `screen.subscription()` only for `Main`; return
  `Subscription::none()` for `Setup`/`Debug`.
- Keep `handle_debug_emit` as-is.

### 5.8 `src/lib.rs`

- Remove `use iced::Subscription` if it becomes unused.
- Remove `pub trait Screen`.
- Keep `Action` and `Never` (engine and child screens still use `Action`).

### 5.9 `src/prelude.rs`

- Remove `Screen` from the re-export.
- Keep `Action` and `Never`.
- Import `Ctx` explicitly as `use crate::screen::Ctx;` in screens that need it
  rather than adding it to the prelude.

### 5.10 Subscriptions

A screen that subscribes exposes the method with screen-layer visibility:

```rust
impl State {
    pub(in crate::screen) fn subscription(&self) -> Subscription<Msg>;
}
```

The parent that owns the screen calls it and maps the messages:

```rust
impl MainScreen {
    pub fn subscription(&self) -> Subscription<Msg> {
        let Some(entry) = self.entries.last() else {
            return Subscription::none();
        };

        match &entry.screen {
            SubScreen::Artists(state) => {
                state.subscription().map(Msg::ArtistsScreenMsg)
            }
            // exhaustive match
        }
    }
}
```

- Use `pub(in crate::screen)`, not `pub`. A `pub` method in the library is
  externally reachable, so `dead_code` will not warn when it is unused.
- Do not add `#[deny(dead_code)]`. The standard warning is the enforcement.
- If a screen has no subscriptions, do not define the method.
- `SubScreen` and the parent match must stay exhaustive so every routed screen's
  subscription is considered.
- `MainScreen::subscription` stays `pub` because `main.rs` is a separate crate
  and must call it from the application wiring.

### 5.11 Placeholder screens

`album_detail.rs`, `artist_detail.rs`, `now_playing_panel.rs`, `queue.rs`, and
`settings.rs`:

- Update the TODOs from `implement Screen for State` to `implement inherent
  init/update/view + Ctx when implemented`.
- No behavior change now.

## 6. Migration order

Keep the tree compiling at each step.

### Phase 0 — Baseline

- `cargo check`
- `cargo check --features dev`
- `cargo test`
- Confirm there are no `dyn Screen`, generic `Screen` bounds, or other trait
  users beyond `impl Screen` and `use Screen`.

### Phase 1 — Add `Ctx`

- Add `Ctx<'a>` to `screen/mod.rs`.
- No call sites change yet.

### Phase 1b — Align dependent change docs

Update the existing plans before implementing anything that follows their old
shape. See §8 for the per-document changes. At minimum:

- Remove statements that `Screen` is unchanged or that screens own
  `Arc<Manager>`/`Arc<PlayerSnapshot>`.
- Replace `impl Screen` snippets with inherent `init/update/view`.
- Replace `Arc` snapshot broadcasting with read-only `Ctx`/engine access.
- Keep feature-specific decisions (panel layout, engine queue events, cover
  ownership) intact unless they depend on the old screen context.

### Phase 2 — Convert `artists` and `MainScreen`

- Remove `manager` from `artists::State`.
- Add inherent `artists::init/update/view`.
- If `artists` has a subscription, make it
  `pub(in crate::screen) fn subscription(...)` and batch it from
  `MainScreen::subscription`.
- Convert `MainScreen` to inherent methods and `Task<Msg>`.
- Update only the `Main` parts of `main.rs` (`LibraryReady`, `ScreenMain`,
  `subscription`, `view`).
- `Setup`/`Debug` can still use the trait temporarily.
- Verify `cargo check` and `cargo check --features dev`.

### Phase 3 — Convert setup and debug

- Convert `library_setup`, `list_libraries`, and `add_offline` to inherent
  methods.
- Convert `debug` to inherent methods.
- Update the remaining `main.rs` arms.
- Apply the same `pub(in crate::screen)` visibility to any setup/debug
  `subscription` methods and ensure their parents batch them.
- Verify again.

### Phase 4 — Delete the trait

- Remove `Screen` from `lib.rs` and `prelude.rs`.
- Remove remaining imports and dead helper methods.
- Update placeholder TODOs.
- `rg "Screen"` should show only prose/comments, no trait uses.

### Phase 5 — Cleanup and validation

- `cargo fmt`
- `cargo check`
- `cargo check --features dev`
- `cargo clippy --all-targets --features dev -- -D warnings` (or the project's
  existing clippy level)
- `cargo test`
- Manual smoke tests:
  - Setup flow: list → add local library → back → open.
  - Main flow: artists load, cover fetch, shuffle.
  - Playback bar: play/pause, seek, volume.
  - `dev` feature: debug screen → back to setup.

## 7. Borrow and lifetime notes

- Construct `Ctx` only immediately before calling a child; do not keep it across
  a mutation of `MainScreen`.
- Destructure `MainScreen` into disjoint fields before creating `Ctx` and
  borrowing `entries` mutably.
- Scope the child-state borrow so the `Action` is owned before handling `Emit`
  with `&mut self`.
- If a future `view` reads `Ctx` and builds widgets from borrowed strings, use:

```rust
pub fn view<'a>(state: &'a State, ctx: Ctx<'a>) -> Element<'a, Msg>;
```

Pass `Ctx` by value (it is `Copy`); the references inside point into
`MainScreen`, so the element can borrow them.

- Do not add a mutable field to `Ctx`. If a child needs a mutation, add an
  `Event` variant and let `MainScreen` apply it.

## 8. Existing change-doc alignment

The screen refactor changes conventions that other plans reference. Those plans
must be updated in the same change (or before the feature they describe is
implemented), otherwise they will reintroduce the old trait/`Arc` model.

### `right_panel_01.md`

This is the main conflicting plan; it is still planned work, not historical.

- Replace §1's **“`Screen` is unchanged”** bullet with the inherent-method
  convention and read-only `Ctx`.
- Delete the `PlayerSnapshot`/`Arc` broadcast model in §2. Panel screens read
  shared state through `Ctx`; they do not own snapshot handles.
- Add queue access without copying it:
  - prefer a borrowed engine accessor,
    `Engine::queue(&self) -> impl ExactSizeIterator<Item = &Track> + '_`, and
    read it as `ctx.engine.queue()`;
  - do not add a separate `Ctx::queue` field unless the accessor proves
    unsuitable; `Ctx` already holds the engine;
  - keep `now_playing: Option<&NowPlaying>` in `Ctx`.
- Replace §3.4’s “No queue accessor needed” with the accessor above.
- Update §4: remove the `snapshot: Arc<PlayerSnapshot>` field; build `Ctx` in
  `MainScreen::update`/`view`; panel states are local UI state only.
- Update §5: `queue::State` no longer stores `Arc<PlayerSnapshot>`; `view` reads
  `ctx.engine.queue()`.
- Update §6: `now_playing_panel::State` no longer stores `Arc<Manager>` or
  `Arc<PlayerSnapshot>`; keep only `cover` and `cover_id`. Use `ctx.manager`
  for cover fetches in `update`.
- Replace `impl Screen` snippets with inherent or module-level
  `init/update/view`.
- Update §9 implementation order, §12 deviations, and any remaining
  `ViewCtx`/`Screen` references.
- Keep the engine queue-event changes, panel layout, cover ownership, and
  long-lived panel-state decisions.

### `library_01.md`

- Replace the locked decision “Screens hold `Arc<Manager>`” with: `MainScreen`
  owns `Arc<Manager>`; children receive `&Arc<Manager>` through `Ctx` or direct
  parameters and never store it.
- Replace the `impl Screen for State` sketch with inherent
  `library_setup::State::init/update/view`.
- Keep `Emit(Arc<Manager>)` as the setup → main handoff.
- Update the wiring notes so `artists::State` no longer holds
  `Arc<Manager>`; cover fetches use the manager passed to `update`.
- Update the `src/screen/mod.rs` / `src/main.rs` summary rows that describe the
  old trait/state shape.

### `engine_01.md`

- Replace `Screen::subscription` with `MainScreen::subscription`.
- Update the child wiring notes (`type Emit`, `Action::handle`) to inherent
  methods and direct `Action` matching.
- Keep the engine ownership, event flow, and `NowPlaying` snapshot decisions.

### `now_playing_02.md`

- No structural conflict: the bar remains `MainScreen` chrome, not a screen.
- Review for `Screen`/`Action::handle` wording and any implication that the bar
  owns an `Arc` snapshot. The bar reads `MainScreen` fields; children receive
  `Ctx`.
- Update terminology to `MainScreen`-owned `now_playing` state where useful.

### `artist_01.md`

- No screen-architecture changes found; review only if the artist detail screen
  is implemented from it.

### Cross-document check

After editing, these patterns should no longer appear in planned screen code
(unless clearly marked as historical):

- `impl Screen`
- `type Ctx`
- `ViewCtx` / `UpdateCtx`
- `Arc<PlayerSnapshot>`
- “Screens hold `Arc<Manager>`”
- “`Screen` is unchanged”

## 9. Out of scope

- Removing `Action` from the engine or changing it to
  `(Vec<EngineEvent>, Task<EngineMsg>)`.
- Implementing the placeholder screens.
- Changing routing/stack behavior or duplicate-route state. When
  `artist_detail` becomes real, store its `State` in the per-entry `SubScreen`
  variant so two `ArtistDetail` entries do not share one state instance.
- Switching Rust channels or changing `devenv.nix`/`flake.nix`.

## 10. Acceptance criteria

- No `Screen` trait remains.
- No child screen stores `Arc<Manager>` or clones live UI state.
- `artists::State` has no `manager` field.
- `MainScreen` is the only owner of `manager`, `engine`, and `now_playing`.
- Children receive `Ctx<'a>` or direct parameters; they never receive
  `&mut MainScreen`.
- Every screen `subscription` is `pub(in crate::screen)` and is called by its
  parent. An unused one produces the normal `dead_code` warning; no
  `#[deny(dead_code)]` is added.
- All mutations flow through `Event`/`Action::Emit`.
- Dependent plans in `change_docs/` are aligned and contain no unmarked old
  `Screen`/`Arc` snapshot instructions.
- Stable Rust only.
- `cargo check`, `cargo check --features dev`, `cargo test`, and the manual smoke
  tests pass.
