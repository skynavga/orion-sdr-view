<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Signal Inputs

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
