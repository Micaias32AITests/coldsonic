# GPUI port — working checklist

Branch goal: rewrite the UI in GPUI (via `gpui-kit`) while the iced app stays
runnable. iced is deleted only at cutover.

## Ground rules

- Two binaries, one library:
  - `coldsonic` — iced, `src/main.rs`, `default-run`.
  - `coldsonic-gpui` — GPUI, `src/bin/coldsonic-gpui.rs` → `coldsonic::gui_app::run()`.
- No direct `gpui` dependency. `gpui-kit` only (currently pinned 0.9.1 +
  `gpui-fast`); mixing backends produces type mismatches.
- Do not port `src/widget/lazy_list.rs`, `src/widget/stack.rs`, or the
  `Screen`/`Ctx`/`Action` machinery. Copy update logic, not view plumbing.
- Do not make the GPUI app depend on `src/screen/*`; it is iced-specific.
- Native deps live in `devenv.nix` / `flake.nix` (`fontconfig`, `freetype`,
  `libxcb`). Re-enter the dev shell before building.
- Checks: `cargo check`, `cargo check --features dev`, `cargo run --bin coldsonic-gpui`,
  `cargo fmt`, `cargo clippy --all-targets --features dev -- -D warnings`.

## Done

- [x] Add GPUI/`gpui-kit` deps and native build inputs.
- [x] Split into two binaries; keep the iced app intact behind `coldsonic`.
- [x] GPUI window, theme, asset source, three-pane static shell.
- [x] `uniform_list` virtualized album rows (dummy data).
- [x] Real `Manager` + `albums()` wired into `Coldsonic`.
- [x] Cover cache: `HashMap<CoverId, Cover>` with `Loading` / `Loaded(ImageSource)` / `Failed`,
      tokio fetch bridged to a GPUI task, `cx.notify()` on completion only.

## Fix TODOs (found in review — do before adding screens)

- [ ] **Cover cache never shrinks.** Encoded `Arc<Image>` plus GPUI's decoded
      `RenderImage` stay pinned for every album ever scrolled past. Start with
      `cover_cache.clear()` on route change, then port `CoverMap::sync`'s
      `retain` against the visible range (see `src/screen/covers.rs`). Release
      decoded pixels with `Image::remove_asset(cx)` once a cover is dropped.

## Port order

### 1. Shell and navigation

- [ ] `MainScreen` entity: owns `Manager`, `Engine`, `now_playing`, route stack,
      panel visibility, and the cover cache.
- [ ] `Route` becomes retained `Entity<...>` children instead of `match` arms in
      one struct (buried-screen state for free — no `stack` widget).
- [ ] `EventEmitter` + `cx.subscribe` for child→parent intents; `cx.listener`
      for self→mutation. Mirror iced's rule: children own local state, the
      parent performs mutations.
- [ ] Sidebar with active-route highlighting; back button.
- [ ] `h_resizable` for the sidebar and right panel. Do this only after content
      looks right — don't debug layout and screen logic at once.
- [ ] Right panel shell (`now_playing_panel` placeholder) and its toggle.

### 2. Engine seam (blocks playback)

- [ ] Remove `iced::Task` from `src/lib.rs`, `src/engine/*`, and the watcher.
      `Engine` must be UI-framework agnostic.
- [ ] Bridge tokio → GPUI with channels (`async_channel` or `tokio::sync::mpsc`)
      instead of iced `Subscription` streams.
- [ ] Keep one tokio `Runtime` for the process; pass a `Handle`, not the
      `Runtime`, once `Services` exists.

### 3. Now Playing and Queue

- [ ] Now-playing bar: cover, title/artist, transport, seek, volume.
- [ ] Tick loop replaces the iced subscription tick (`cx.spawn` timer +
      `cx.notify()`), no UI-thread blocking.
- [ ] `seeking` draft so the 250 ms tick doesn't yank the thumb.
- [ ] Queue screen reuses the virtualized row pattern.

### 4. Remaining screens (same pattern: entity + `uniform_list` + covers)

- [ ] Artists
- [ ] All Tracks
- [ ] Artist Detail
- [ ] Album Detail
- [ ] Settings
- [ ] Library Setup (list / add local / open) — must work without a `Manager`
- [ ] Debug (`dev` feature only)
- [ ] Playlists (currently a placeholder screen)

### 5. Cutover

- [ ] Make `coldsonic-gpui` the default binary (`default-run`, rename).
- [ ] Delete `src/screen`, `src/widget`, and the iced `src/main.rs`.
- [ ] Remove `iced`, `iced_fonts`, `iced_futures` and the `image::Handle` uses.
- [ ] Update `AGENTS.md` and `change_docs/` (screen architecture section is
      iced-specific and will be replaced by the entity model).
- [ ] Confirm `cargo test` still covers manager/engine logic.

### 6. Finish

- [ ] `cargo fmt`, `cargo clippy --all-targets --features dev -D warnings`.
- [ ] Manual pass: scroll performance, resize, missing/corrupt covers, no library
      configured, library open failure.
- [ ] Nix build (`nix build` / devenv) without the manual `PKG_CONFIG_PATH`.

## Notes / pitfalls

- GPUI executor is not tokio. Any sqlx/reqwest future must be `spawn`ed on the
  tokio runtime; only the continuation runs in `cx.spawn`.
- `uniform_list`'s closure is a render pass with `&mut Self` via `cx.processor`.
  Mutate state there (e.g. kick off a fetch) but never call `cx.notify()` —
  that loops.
- `ImageSource::Image(Arc<Image>)` decodes lazily in GPUI's asset cache, keyed by
  a hash of the bytes. Build it once per cover, not per frame.
- `Manager` is not `Clone`; use `manager.handle()` (`ManagerHandle`) for tasks.
- `RenderImage` is not `Clone`; wrap in `Arc` or let the asset cache own it.
- Borrowing: clone small ids (`CoverId`) before calling `&mut self` helpers from
  inside a `uniform_list` closure; clone theme colors (`Hsla` is `Copy`) instead
  of holding `cx.theme()` across a listener/processor call.
- `gpui_kit::init(cx)` must run before any component, and
  `.with_assets(assets::Assets)` must be set, or icons/themes fail.
