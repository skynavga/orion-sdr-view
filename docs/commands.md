<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Build and Test Commands

## Building

```sh
# Debug build
cargo build

# Release build
cargo build --release

# Build and run the viewer
cargo run --release
```

## The two feature configurations

The GUI dependencies (`eframe`, `egui`) sit behind an optional `gui` feature, **enabled by
default**. `--no-default-features` builds the DSP library alone, with no windowing system:

```sh
# Everything: library, binary, GUI, replay driver
cargo build --release

# Library only — no eframe, no egui, no binary
cargo build --release --no-default-features
```

**CI builds and tests both, and that is not redundant.** It once checked only
`--no-default-features`, which meant nothing under `src/app/` was compiled by CI at all — a
GUI-side type error could merge green. Roughly a fifth of the tests are behind `gui`, and
they are not incidental ones: every settings-row and key-binding test, the whole replay
driver, and the link-budget harness. None of them needs a display or a GPU, because the
harness drives `egui::Context` directly and never opens a window.

## Testing

```sh
# Everything (the default feature set includes gui)
cargo test --release

# Library only, the way a headless runner sees it
cargo test --release --no-default-features
```

Use `--release` for anything that measures. Debug builds run the DSP an order of magnitude
slower, which turns a link-budget sweep from a minute into a coffee break and makes any
throughput figure meaningless.

A few tests are `#[ignore]`d because they are measurement harnesses rather than assertions —
`tests/cofdm_link_budget.rs` sweeps C/N per bandwidth fraction and prints FER and EVM tables.
Neither CI step passes `--ignored`, so they are compiled on every run and executed only when
asked. That is the useful half: a refactor that breaks the harness still fails the build.

```sh
cargo test --release --test cofdm_link_budget -- --ignored --nocapture --test-threads=1
```

## Linting

What CI runs, in order — worth running locally before pushing:

```sh
cargo fmt --check
cargo check --no-default-features
cargo clippy --no-default-features --tests -- -D warnings
cargo check --features gui
cargo clippy --features gui -- -D warnings
```

## Running headless

The viewer also runs with no window, renderer or GPU, driven by a timed key script. See
[headless.md](headless.md) for the script format and the dump it writes:

```sh
cargo run --release -- --headless --script scripts/cofdm-link.txt --dump run.jsonl
```
