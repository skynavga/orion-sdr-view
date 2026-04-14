# CLAUDE.md — orion-sdr-view

## Project

`orion-sdr-view` is a keyboard-driven SDR spectrum visualization tool in Rust (crate name `orion-sdr-view`, bin name `orion-sdr-view`, v0.0.11), edition 2024, built on egui/eframe. Displays live spectrum, persistence density, and a cycle-able waterfall pane (vertical waterfall or horizontal spectrogram, toggled with `W`) from a configurable signal source.

## Conventions

- **Copyright headers**: every tracked `.rs` file begins with

  ```
  // Copyright (c) 2026 G & R Associates LLC
  // SPDX-License-Identifier: MIT OR Apache-2.0
  ```

  Markdown files (`README.md`, `CHANGELOG.md`) carry the same notice wrapped in an HTML comment above the top-level heading. `.claude/**` and `CLAUDE.md` are exempt. When creating new source files, include the header.
