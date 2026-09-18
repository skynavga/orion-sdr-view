<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

<!-- markdownlint-disable MD013 -->

# CLAUDE.md — orion-sdr-view

## Project

`orion-sdr-view` is a keyboard-driven SDR spectrum visualization tool in Rust ed. 2024, built on egui/eframe. Displays 1 to 3 panes: live spectrum, persistence density, and a cycle-able vertical waterfall → horizontal spectrogram → constellation + correction map from a configurable signal source.

## Essential Reading

- [Conventions](docs/conventions.md) — code, testing, documentation, git

## Project Docs (Read as Needed)

- [Build and test commands](docs/commands.md) — cargo aliases, running the app
- [Capture](docs/capture.md) - images and video
- [COFDM source](docs/cofdm.md) — wideband coded OFDM source design
- [Configuration](docs/configuration.md) — `.orionsdr.yaml` schema and precedence
- [Headless replay](docs/headless.md) — scripted, GPU-less runs for measurement capture
- [Impairment](docs/impairment.md) — `cn_db` carrier-to-noise model shared by all sources
- [Signal inputs](docs/inputs.md) — how to add a new signal source
- [Keyboard shortcuts](docs/shortcuts.md) — full key reference
- [Source layout](docs/source.md) — module tree and per-source file conventions
- [Terminology](docs/terminology.md) — acronyms and glossary
- [Viewport](docs/viewport.md) — zoom and panning semantics
