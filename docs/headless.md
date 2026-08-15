<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Headless replay

```sh
orion-sdr-view --headless --script demo.txt --dump run.jsonl --duration 30
```

Runs the viewer with **no window, no renderer and no GPU**, driven from a timed key script at a
fixed frame delta, and writes the measurement stream the `Di` bar and the `X` panel consume as
machine-readable records.

## The script format

Plain text, times in absolute seconds:

```text
duration 30                    # run settings: no time column, at most one each
dump     run.jsonl

# t(s)   directive
0.00     source COFDM          # select a source by name
0.50     key ArrowUp x3        # zoom in
1.00     key L                 # lock the band to the viewport centre
1.50     text a                # markers arrive as Text, not Key
2.00     assert center_hz 520000
```

| Directive | What it does |
| --- | --- |
| `key <[mod+]Name>` | Press and release one key within a single pass. Modifiers are spelled out: `shift+`, `ctrl+`, `alt+`, `cmd+` |
| `source <name>` | Select a source by name, case- and punctuation-insensitively |
| `text <literal>` | Deliver a text event — the only way to reach the marker, help and dB-reference bindings |
| `assert <name> [args]` | A property for the *test harness* to check; the replay driver parses it and ignores it |

A repeat count is **frames, not events**: `key_pressed` is a per-pass boolean, so five press events
inside one pass register as one. `assert` directives are parsed — a typo is still an error — and
then ignored; executing them is the test suite's job. It is one format with two readers, so a
reproduction recipe and a regression test are the same file.

### Naming a source

`source COFDM` is not a shortcut past the UI. It presses `I` exactly as `key I` does, and keeps
pressing until the named source is active — **the count is worked out at run time** rather than
written down. From a default start, `source COFDM` and `key I x5` produce identical runs.

What the name removes is the count, and the count is the part that goes stale. `key I x5` encodes
the *distance* from wherever the app happens to be to the source you meant, so adding a source,
reordering the list, or starting from a different one retargets every such line at once. It fails
silently, too: the line still parses, still runs, and the dump it produces is a perfectly valid
measurement of the wrong source.

Names are case- and punctuation-insensitive, and may be written with the spaces the label has, so
`AM DSB`, `AM-DSB`, `AM_DSB` and `amdsb` are one source. A name that matches nothing is a **parse
error** — the run stops before it starts, naming the line and listing the sources that do exist:

```text
orion-sdr-view: script: line 12: `COFDMM` is not a source
  (expected one of: Test Tone, CW, AM DSB, PSK31, FT8, COFDM)
```

Fatal rather than a warning, for the same reason: a skipped `source` would leave the run measuring
whatever was already active, and nothing downstream could tell.

Naming the source that is already active does nothing at all — no presses, no reset. Re-selecting is
not free, since it flushes the decode pipeline and restarts the burst.

## Run settings, and what overrides what

`duration` and `dump` take no time column, because they configure the run rather than happen during
it. They let a script be a complete recipe — what to press, how long for, and where the answer goes
— instead of a file that needs a remembered command line beside it.

**The command line overrides either**, which is what keeps that recipe reusable: the same script can
be run longer or written elsewhere without being edited. Paths are taken verbatim, so a relative
`dump` resolves against the working directory exactly as `--dump` does.

With neither naming a value:

- **no duration** → the run ends **one second past the last scripted step**. Without that margin it
  would stop on the very frame the last action lands on, and whatever that action was for would
  never be measured.
- **no dump** → nothing is written. The run is still worth doing: it fails on a panic, an
  unparsable script or a dropped chunk just the same. A dump of `-` writes to stdout instead; see
  [below](#dumping-to-stdout).

`--duration` may also be shorter than the script. Every step still runs — the loop waits on the
script as well as the clock — so cutting a run short cannot silently skip the actions that were
asked for.

### Dumping to stdout

**A dump path of `-` means standard output**, the same spelling `curl -o -` and `tar -f -` use, in
both `--dump` and the script's own `dump` directive:

```sh
orion-sdr-view --headless --script scripts/cofdm-link.txt --dump - \
  | jq -r 'select(.kind=="instrument") | "\(.t)  MER \(.mer_db.v)"'
```

This works because **nothing else in a headless run writes to stdout** — the frame/sample summary
and every diagnostic go to stderr — so the stream stays parseable with no filtering. Output is line
buffered rather than block buffered, so a reader downstream gets a record at a time instead of the
whole run at the end.

A file genuinely called `-` is still reachable as `./-`, which is the escape hatch the same
convention offers everywhere else. Only the whole path counts: `runs/-`, `dash-` and `-.jsonl` are
ordinary files.

## Example scripts

Ready-to-run recipes live in [`../scripts/`](../scripts/), with a one-line description of each in
[`../scripts/README.md`](../scripts/README.md): measuring a COFDM link, breaking one below the FEC
cliff to see the `null` readings, walking every source, CW decode with its burst timestamps, and a
viewport reproduction recipe built from `assert` directives.

**The same script produces the same bytes.** Both pseudo-random generators are seeded from fixed
constants, and a replay run removes the four impure reads that would otherwise vary: the frame
clock is injected, the decode runs inline rather than on a worker thread, no chunk can be dropped,
and timestamps come from a scripted clock starting at 2026-01-01T00:00:00Z rather than from the
system one. A run therefore reads `|| 00:00:00.033 |` for a burst 33 ms in — legible as elapsed
time and impossible to mistake for a real one.

## The dump

One JSON object per line. JSON Lines rather than CSV because every instrument reading is an
`Option` with a provenance:

```json
{"kind":"header","version":"0.0.26","source":"Test Tone","fs_hz":48000.0,"script_sha256":"3072e5…"}
{"kind":"source","t":0.083,"samples":4000,"source":"COFDM","fs_hz":1920000.0}
{"kind":"info","t":0.483,"samples":102304,"modulation":"COFDM","center_hz":480000.0,"bw_hz":240000.0,"snr_db":34.5}
{"kind":"instrument","t":0.483,"samples":102304,"cn_db":{"v":34.5,"prov":"measured"},"clock_error_ppm":{"v":null,"prov":"unavailable"},…}
{"kind":"summary","t":3.0,"frames":180,"samples":720800,"dropped_chunks":0,"records":20}
```

`null` and `0.0` must stay distinguishable: the BER rungs go absent exactly when the link fails, so
a format that could not hold `null` would render a dead link as a flawless one. `prov` is the same
distinction the panel's `SIM` badge draws — `measured` came off the air, `known` was declared by the
source, `unavailable` has no provider at all.

**`t` is scripted time, not signal time.** The per-frame sample budget is `dt × fs` clamped to 4096,
and at COFDM's 1.92 MHz that clamp binds hard — a 1/60 s frame asks for 32 000 samples and gets
4096. Every record therefore also carries `samples`, the cumulative count actually consumed, which
is the honest measure. `dropped_chunks` in the summary must be zero; a hole breaks a streaming
demodulator's framing, so anything else means the dump measured the harness rather than the link.

A headless run **fails loudly** — non-zero exit and a message on stderr — for an unparsable script,
an unwritable dump, or a dropped chunk. Nobody is watching it, so a quiet failure would look exactly
like a clean run.
