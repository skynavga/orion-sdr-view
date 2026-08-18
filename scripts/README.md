<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Example replay scripts

Timed key scripts for `orion-sdr-view --headless`.  Each is a **self-contained
recipe**: what to press, how long to run, and — where it makes sense — where the
answer goes.

```sh
orion-sdr-view --headless --script scripts/cofdm-link.txt --dump run.jsonl
```

None of these names a dump, deliberately, so running one from a checkout writes
nothing into the tree.  Add `--dump FILE`, or a `set run.dump FILE` line, when
you want the measurement stream.  Anything on the command line overrides the
script.

| Script | What it is for |
| --- | --- |
| `cofdm-link.txt` | Measure a COFDM link: instrument records at the default C/N |
| `cofdm-degraded.txt` | The same link at a C/N low enough to break it, to see `null` readings |
| `every-source.txt` | Walk all six sources; a smoke test and a tour of the dump's record kinds |
| `cw-decode.txt` | CW text decode, with the scripted clock's burst timestamps |
| `cofdm-continuous.txt` | A link that never gaps, for a long measurement |
| `viewport.txt` | Zoom, lock and pan — a UI reproduction recipe rather than a measurement |
| `overscan.txt` | Pan past the band edge and back, with a `still` at each stop |

Two of these need a source parameter the defaults do not give — a C/N below the
FEC cliff, a burst that never ends — and each states it with an untimed `set`
rather than the `--config` file it used to need beside it:

```text
set cofdm.cn_db  5.0          # cofdm-degraded.txt
set cofdm.sig_secs 1.0e9      # cofdm-continuous.txt
```

The format is documented in [`docs/headless.md`](../docs/headless.md).

## Regenerating the README screenshots

`docs/images/*.png` are headless captures, not hand-taken screenshots, so they can be
brought back up to date in one command rather than drifting until someone notices:

```sh
scripts/docs/regen-docs-images.sh                  # all of them, in place
scripts/docs/regen-docs-images.sh source-am-dsb    # just one
scripts/docs/regen-docs-images.sh --out /tmp/shots # elsewhere, to compare first
```

One recipe per image in [`docs/`](docs/), matched by name and nothing else — `<name>.txt` is
the script, its `still` label is `<name>`, and `docs/images/<name>.png` is what lands.  Adding
an image to the README is adding one file there; the driver needs no edit.

| Recipe | The picture it takes |
| --- | --- |
| `source-am-dsb.txt` | AM-DSB on voice audio, mid-burst |
| `source-cofdm-instrumented.txt` | COFDM with the `X` panel up, every field measured |
| `source-cofdm-constellation.txt` | Pane 3's constellation and correction map, at 10 dB C/N on a source that never gaps |

Each states its own `set run.size 1200x828` and `set run.scale 2`, which is what makes every
image 2400 x 1656 at 72 dpi; a run that produces some other size is reported rather than
quietly committed.  Everything else a recipe needs it states with `set` as well, so there is
nothing beside it to remember.

**Reproducible run to run, not machine to machine.** Two runs here give byte-identical PNGs.
A different architecture will not: `rustfft` dispatches AVX on x86-64 and Neon on AArch64, so
the spectrum differs in its last bits and every pixel downstream inherits it.  Regenerating on
another machine rewrites all three files even when nothing has changed.
