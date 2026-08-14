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

None of these names a `dump`, deliberately, so running one from a checkout
writes nothing into the tree.  Add `--dump FILE`, or a `dump FILE` line, when you
want the measurement stream.  Anything on the command line overrides the script.

| Script | What it is for |
| --- | --- |
| `cofdm-link.txt` | Measure a COFDM link: instrument records at the default C/N |
| `cofdm-degraded.txt` | The same link at a C/N low enough to break it, to see `null` readings |
| `every-source.txt` | Walk all six sources; a smoke test and a tour of the dump's record kinds |
| `cw-decode.txt` | CW text decode, with the scripted clock's burst timestamps |
| `cofdm-continuous.txt` | A link that never gaps, for a long measurement |
| `viewport.txt` | Zoom, lock and pan — a UI reproduction recipe rather than a measurement |

Two of these need a *config* setting rather than a script one, so each comes
with a matching YAML file — `degraded.yaml` and `continuous.yaml`:

```sh
orion-sdr-view --headless --config scripts/degraded.yaml \
  --script scripts/cofdm-degraded.txt --dump run.jsonl
```

The format is documented in the top-level `README.md` under **Headless replay**.
