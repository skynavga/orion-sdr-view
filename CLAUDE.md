# CLAUDE.md — orion-sdr-view

## Project

`orion-sdr-view` is a keyboard-driven SDR spectrum visualization tool in Rust
(crate name `orion-sdr-view`, bin name `orion-sdr-view`, v0.0.13), edition
2024, built on egui/eframe. Displays live spectrum, persistence density, and
a cycle-able waterfall pane (vertical waterfall or horizontal spectrogram,
toggled with `W`) from a configurable signal source.

## Conventions

- **Copyright headers**: every tracked `.rs` file begins with

  ```text
  // Copyright (c) 2026 G & R Associates LLC
  // SPDX-License-Identifier: MIT OR Apache-2.0
  ```

  Markdown files (`README.md`, `CHANGELOG.md`) carry the same notice wrapped
  in an HTML comment above the top-level heading. `.claude/**` and `CLAUDE.md`
  are exempt. When creating new source files, include the header.

- **`mod.rs` files contain only `pub mod` / `pub use`**.  Logic lives in a
  sibling `common.rs` (or other named submodule).  Adding code to a `mod.rs`
  is a layout violation; move it.

## Layout

Source-specific code lives in per-source directories:

- **Lib (`src/source/<S>/`)** — one directory per signal source.  Files:
  `source.rs` (the `<S>Source` impl + `<S>Source::apply_params`), `decode.rs`
  (decode worker state), `config.rs` (YAML schema + defaults).  No UI
  dependencies.
- **Bin app glue (`src/app/source/<S>.rs`)** — settings-to-source mapping:
  `make()`, `sync()`, `apply_message()`, HUD formatters, and a `Factory` ZST
  implementing `SourceFactory`.  Depends on
  `crate::app::settings::<S>Settings`.
- **Bin settings (`src/app/settings/<S>.rs`)** — `<S>Rows` row container +
  `impl SourceRows for <S>Rows` + per-source typed-accessor trait
  `<S>Settings` + `impl <S>Settings for SettingsState`.

Cross-source dispatch is **trait-based, not match-based**:

- `SourceRows` (`app/settings/common.rs`) — uniform settings UI surface.
  `SettingsState::sources: Vec<Box<dyn SourceRows>>` holds the per-source row
  containers, indexed by `SourceMode as usize`.  Every settings dispatch in
  `common.rs` is a single trait call on `self.active_source()` — no
  per-source `match`.
- `SourceFactory` (`app/source/common.rs`) — uniform source orchestration.
  `app::source::FACTORIES: &[&dyn SourceFactory]` is a static dispatch table
  indexed by `SourceMode as usize`.  `app/sources.rs::make_source` and
  `sync_decode_config` dispatch through `source_mode_factory(mode).method(...)`
  — no per-source `match`.
- Per-source typed accessors (`<S>Settings` traits) provide
  `self.settings.cw_wpm()`-style typed access via
  `SettingsState::source_as::<T>(idx)` downcast.

## Adding a new signal source

Eight steps — none of them touch `app/settings/common.rs` or
`app/sources.rs`:

1. Lib: create `src/source/<S>/{source,decode,config}.rs` and `mod.rs`
   (re-exports).
2. Lib: register `pub mod <s>;` in `src/source/mod.rs`; add per-source
   decode/config re-exports to `src/decode/mod.rs` and `src/config/mod.rs`
   if the source needs them.
3. App glue: create `src/app/source/<S>.rs` with `make()`, `sync()`,
   message-commit functions, and a `pub(super) struct Factory; impl
   SourceFactory for Factory { ... }` block.
4. App glue: register `pub(super) mod <s>;` in `src/app/source/mod.rs` and
   push `&<s>::Factory` into the `FACTORIES` table in
   `src/app/source/common.rs`.
5. Settings: create `src/app/settings/<S>.rs` with `<S>Rows` (rows + `impl
   SourceRows`) and an `<S>Settings` typed-accessor trait + impl.
6. Settings: register `mod <s>;` in `src/app/settings/mod.rs` and add
   `pub(in crate::app) use <s>::<S>Settings;` to the trait re-export block.
7. Settings storage: in `app/settings/common.rs::SettingsState::new()`,
   push `Box::new(<S>Rows::new())` into `sources` (one line, in
   source-mode-index order).
8. `SourceMode` enum: add the new variant to `src/app/common.rs` and update
   `SourceMode::ALL`, `label()`, and the `source_selector` toggle options in
   `SettingsState::new()`.

`app/settings/common.rs` and `app/sources.rs` should not require edits —
both dispatch through traits.  The selector toggle options array is the
only "list everything" spot in the bin.
