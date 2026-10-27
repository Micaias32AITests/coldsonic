# AGENTS.md

## Project

Coldsonic is an iced music player. The UI is hierarchical: `MainScreen` owns the
shared application state and coordinates the screens in the navigation stack.

Read these before changing screen code:

- `change_docs/screen_01.md` — the screen/context refactor plan and rationale.
- The module documentation at the top of `src/screen/mod.rs`.
- The relevant feature doc in `change_docs/` (for example `right_panel_01.md`).

## Commands

Prefer the repository's Nix/devenv environment if the system toolchain does not
match.

```sh
cargo check
cargo check --features dev
cargo test
cargo fmt
cargo clippy --all-targets --features dev -- -D warnings
```

## Screen architecture

There is no `Screen` trait and no associated screen context. Screens are plain
state structs with inherent methods or module functions.

The `Screen` impls still present during the migration are leftovers. Do not add
new `impl Screen`, and do not add a new associated `Ctx`/GAT/nightly feature.

### Ownership

- `MainScreen` owns shared state: `manager`, `engine`, `now_playing`, routing,
  layout, and persistent UI settings.
- A screen's `State` holds only local UI state. It must not store
  `Arc<Manager>`, `Engine`, or any other parent-owned value.
- Shared read-only state is passed down as a named `Ctx<'a>` when several values
  are needed, or as direct parameters when only one or two are needed.
- `Ctx` is `Copy`, read-only, built immediately before the call, and never
  stored in a screen. Add a field only when a screen reads it. If you want to
  add a field, tell the user about it and stop until you have explicit
  permission to do it.
- Do not add mutable fields to `Ctx`. Mutations go upward as events.

### Events

- A screen returns `Action<Msg, Event>` from `update`.
- `Event` is the screen's intent for its parent. `MainScreen` handles it and
  performs the mutation.
- Never pass `&mut MainScreen` into a screen.
- When an event needs a manager for an async task, clone the `Arc` handle into
  the task. This is a refcount bump, not a clone of live UI state.
- If a screen needs a value that is not in `Ctx`, add an event or add a
  read-only `Ctx` field. Do not give the screen its own copy of shared state.

### Subscriptions

- A screen that subscribes exposes
  `pub(in crate::screen) fn subscription(&self) -> Subscription<Msg>`.
- The parent must batch it. If the parent forgets, the normal `dead_code`
  warning flags the unused method. Do not add `#[deny(dead_code)]`; the warning
  is the enforcement.
- Do not make a child screen's `subscription` `pub`. Public items are externally
  reachable and suppress the `dead_code` warning.
- `MainScreen::subscription` stays `pub` because `main.rs` is a separate crate
  and must call it from the application wiring.
- Keep the parent `match`/registry exhaustive so every routed screen gets a
  subscription decision.

## Adding a screen

1. Create `src/screen/<name>.rs` and add `pub mod <name>;` to
   `src/screen/mod.rs`.
2. Define the screen's local types:

   ```rust
   pub struct State { /* local UI state only */ }

   pub enum Msg { /* input and async results */ }

   pub enum Event { /* intents for MainScreen */ }
   ```

3. Provide inherent methods. Use the exact parameters the screen needs:

   ```rust
   pub fn init(/* screen-specific data */) -> (State, Task<Msg>);

   // If shared read-only state is needed:
   pub fn update(
       state: &mut State,
       ctx: Ctx<'_>,
       msg: Msg,
   ) -> Action<Msg, Event>;

   // If only one or two values are needed, use direct parameters instead.
   pub fn update(
       state: &mut State,
       manager: &Arc<Manager>,
       msg: Msg,
   ) -> Action<Msg, Event>;

   // Take Ctx only when the view reads shared state. Otherwise omit it.
   pub fn view<'a>(state: &'a State, ctx: Ctx<'a>) -> Element<'a, Msg>;

   // Only if the screen has subscriptions:
   pub(in crate::screen) fn subscription(&self) -> Subscription<Msg>;
   ```

4. Keep `State` transitions in `update`. `view` must not mutate state and must
   not clone shared state. Batch any `subscription` from the parent.
5. Register the screen in:
   - `SubScreen` in `src/screen/mod.rs`;
   - `screen::Msg` if it needs a message variant;
   - `MainScreen::update` for dispatch;
   - `MainScreen::view_route` for rendering;
   - `MainScreen::make_screen` for construction, if it is a routed screen.
6. If the screen can appear more than once in the navigation stack, store its
   `State` in its `SubScreen` variant. Do not store only an id and keep one
   shared `State` on `MainScreen`, or duplicate routes will overwrite each
   other.
7. For panel screens, register them with the panel enum/state instead of the
   navigation stack.
8. Replace the placeholder TODO in the screen file and update the relevant
   `change_docs/` plan if behavior or data flow changed.
9. Run the checks above and manually exercise the screen in both normal and
   `dev` builds when applicable.

## Borrowing rules

- Destructure `MainScreen` into disjoint fields before creating `Ctx` and
  borrowing `entries` mutably.
- Build `Ctx` only immediately before calling the child. Do not hold it across a
  later `&mut self` operation.
- Scope the child-state borrow so the returned `Action` is owned before
  `MainScreen` handles `Event` with `&mut self`.
- If a view builds widgets from strings borrowed through `Ctx`, tie the lifetime
  explicitly:

  ```rust
  pub fn view<'a>(state: &'a State, ctx: Ctx<'a>) -> Element<'a, Msg>;
  ```

## Do not

- Add new `impl Screen` blocks.
- Add a trait just to unify screen signatures.
- Add GATs, associated type defaults, or switch to nightly for screen context.
- Store `Arc<Manager>` or `Engine` in a screen `State`.
- Send `&mut MainScreen`, `&mut Engine`, or `&mut UiSettings` into a child.
- Clone live UI values to work around the borrow checker.
