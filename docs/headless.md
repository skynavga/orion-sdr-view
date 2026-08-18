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
set run.duration 30            # untimed: how the run is conducted
set run.dump     run.jsonl
set cofdm.cn_db  10            # untimed: a settings row, before frame 0

# t(s)   directive
0.00     source COFDM          # select a source by name
0.50     key ArrowUp x3        # zoom in
1.00     key L                 # lock the band to the viewport centre
1.50     text a                # markers arrive as Text, not Key
2.00     set cofdm.cn_db 5     # ...and timed, as an edit during the run
2.00     assert center_hz 520000
```

**One rule tells the two shapes apart**: a line beginning with `set` is untimed, and every other
line begins with a time.

| Directive | What it does |
| --- | --- |
| `key <[mod+]Name>` | Press and release one key within a single pass. Modifiers are spelled out: `shift+`, `ctrl+`, `alt+`, `cmd+` |
| `source <name>` | Select a source by name, case- and punctuation-insensitively |
| `text <literal>` | Deliver a text event — the only way to reach the marker, help and dB-reference bindings |
| `set <scope>.<key> <value>` | Write a run setting or one of the app's settings rows; see [below](#set) |
| `still [label]` | Capture the whole window to the capture directory |
| `pane <name> [label]` | Write one pane's raster to the capture directory |
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

## `set`

One directive over three scopes, and the scope says which kind of thing is being written.

### `run.` — how the run is conducted

| Setting | Meaning |
| --- | --- |
| `set run.duration <secs>` | How long to run |
| `set run.dump <path>` | Where the measurement stream goes; `-` is stdout |
| `set run.capture <dir>` | Where `still` and `pane` captures are written |
| `set run.size <W>x<H>` | Logical window size, in points |
| `set run.scale <n>` | Pixels per point; `2` is a Retina-class display |

These take no time column, because they configure the run rather than happen during it — a `run.`
key with a time is a parse error. They let a script be a complete recipe — what to press, how long
for, and where the answer goes — instead of a file that needs a remembered command line beside it.

**The command line overrides every one of them**, which is what keeps that recipe reusable: the same
script can be run longer or written elsewhere without being edited. Paths are taken verbatim, so a
relative `dump` resolves against the working directory exactly as `--dump` does.

### `display.` and a source name — the app's own settings

The rows the `S` popover shows, named as [the config file](configuration.md) names them:

```text
set cofdm.cn_db     10        # sources.cofdm.cn_db
set cofdm.sig_secs  1.0e9     # ...and its Signal row reads `cont`
set display.db_min  -90       # display.db_min
```

A source may be spelled as the config writes it or as the HUD shows it — `am_dsb` and `AM DSB` fold
alike, so there is one spelling of a source in this format rather than one per directive. A toggle
takes its option as shown or any unambiguous prefix, which is why the `Mask` row's `60 dB` is
reachable as the config's `60`.

**Untimed it is a configuration; timed it is an interaction.** Untimed, a `set` is applied before the
first frame and moves the row's *default* as well as its value, so an `R` reset returns to it —
exactly what `--config` does, and the reason three example recipes no longer carry a YAML file
beside them. Timed, it is applied at that instant and moves the value only, so a reset discards it
like any other edit. That is also what makes a sweep expressible:

```text
set run.duration 40
set cofdm.cn_db 20.0          # start above the FEC cliff
0.00   source COFDM
10.00  set cofdm.cn_db 12.0
20.00  set cofdm.cn_db  8.0   # walk it down
30.00  set cofdm.cn_db  5.0
```

One run and one dump for the whole cliff, where before it was one run per point.

### What `set` deliberately cannot reach

**A row, not a config field.** A `set` writes the settings row a popover edit writes, and is read
back by the same accessors — so it cannot reach a state no user could. A test drives the same C/N
change twice, once by opening the popover and nudging and once by naming the value, and requires the
two measurement streams to agree.

Two consequences look like omissions and are not. `cofdm.fs_hz` is a config key with **no row** — a
live sample rate would re-derive Nyquist underneath the viewport — so it stays a `--config` key and
naming it in a `set` is an error rather than a silent no-op. And the rows with no config key, such as
AM-DSB's audio selection and the message-mode toggles, are reached with `key` and `text` instead.
Between the two halves every row is reachable, each exactly once.

A key that does not exist is a **parse error** listing the ones that do, and a *value* no row will
take stops the run before the first frame — timed ones included, since a misspelled toggle option
thirty seconds in would otherwise waste thirty seconds before saying so. A value outside a row's
range is **clamped rather than refused**, because that is what the row does to a nudge; the clamp is
reported on stderr rather than applied silently.

### With no value named

- **no duration** → the run ends **one second past the last scripted step**. Without that margin it
  would stop on the very frame the last action lands on, and whatever that action was for would
  never be measured.
- **no dump** → nothing is written. The run is still worth doing: it fails on a panic, an
  unparsable script or a dropped chunk just the same. A dump of `-` writes to stdout instead; see
  [below](#dumping-to-stdout).

### Size and scale

A headless pass has no window, so it supplies no `screen_rect` — and egui's fallback for one that
does not is **10000 x 10000 at scale 1**, a size no window has. The driver therefore states one:
1200 x 828 at scale 1, the interactive window's own size, so a scripted reproduction lays out the
way a user's session does.

Nothing consults the layout while a run only advances the DSP and handles keys, so this changes no
measurement — pinned by a test that runs the same script at two sizes and compares the dumps. It
matters to anything that *draws*.

`--duration` may also be shorter than the script. Every step still runs — the loop waits on the
script as well as the clock — so cutting a run short cannot silently skip the actions that were
asked for.

### Dumping to stdout

**A dump path of `-` means standard output**, the same spelling `curl -o -` and `tar -f -` use, in
both `--dump` and the script's own `set run.dump`:

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

## Capturing the window

`still` captures everything the viewer draws — HUD, spectrum, panes, decode bar and overlays — with
no window, no renderer and no GPU:

```text
set run.capture  ./shots
set run.size     1200x828
set run.duration 8

0.00 source COFDM
1.00 key D
5.00 still cofdm
```

**The frame is rasterized on the CPU**, and that is a deliberate choice over rendering it on a GPU.
A GPU render cannot promise the same bytes twice — fill rules and texture filtering vary by vendor
and driver version — and a capture that cannot be reproduced is no use as a test fixture, because a
difference could not be told from a different machine. Two runs of one script produce
**byte-identical PNGs**, which a test pins.

What makes that cheap to guarantee is what egui already does. Its preferred framebuffer is *not*
sRGB, so the fragment stage is the whole of `vertex_colour × texture_sample` with no colour-space
conversion — no `powf`, and therefore no libm transcendental whose result can differ between x86
and ARM. Anti-aliasing is already carried in the geometry, since epaint feathers edges by emitting
extra triangles, and MSAA is off; so coverage is one sample per pixel centre. Every operation is
`+`, `-`, `*`, `/` or a comparison: IEEE-correctly-rounded, and identical everywhere.

The CPU renderer is checked against the real GPU pipeline by
`tests/raster_oracle.rs`, which renders the same primitives through `egui-wgpu` offscreen: on a
full COFDM window, 1.2% of pixels differ at all with a worst channel delta of 2 of 255 — edge
coverage on feathered text, and no systematic error.

**Reproducible across runs, not across architectures.** Two runs on one machine give identical
bytes. Two *different* machines may not, and the rasterizer is not why: `rustfft` dispatches to AVX
on x86-64 and Neon on AArch64, so the spectrum itself differs in its last bits and every pixel
downstream inherits that. A committed golden image of a captured frame will therefore fail when
the architecture changes. Compare within one architecture, or assert on regions and statistics
rather than whole images.

### It costs nothing unless a script asks for it

A run whose script contains no `still` **never draws and never tessellates**. The decision is made
once, before the loop, from the parsed script; without one the driver builds no rasterizer at all
and behaves exactly as it did before the feature existed. A test compares the dumps of two such
runs to keep it that way.

Drawn frames are the expensive ones, so capture at moments rather than continuously. There is no
video path here for that reason.

## Capturing a pane

`pane waterfall` writes one pane's raster to the capture directory, as a PNG with a metadata
sidecar beside it:

```text
set run.capture  ./shots
set run.duration 8

0.00 source COFDM
5.00 pane waterfall locked
5.00 pane spectrogram
```

**No renderer is involved, which is why this works headless at all.** Every capturable pane —
`waterfall`, `spectrogram`, `persistence`, and the decoder mode's `constellation` and `correction`
— keeps its pixels CPU-side, which is what makes their ring arithmetic assertable without a GPU,
and a `pane` capture reads those buffers in the same display order the painter uses. So it is the
DSP's own output, without the HUD, the spectrum plot or any chrome around it: a cheaper thing than
a picture of the window, and a different question.

The spectrum pane is absent deliberately. It is a line plot drawn straight to a painter, with no
pixel buffer to hand over — and the reverse of that reasoning is why the constellation is *stamped*
into a raster rather than drawn as painter circles: it would otherwise be uncapturable for the same
reason.

`constellation` and `correction` write nothing until the receiver has decoded a frame, since an
empty pane has no pixels. They need a COFDM source with pane 3 in the decoder mode (`W` twice from
the default); the driver reports "the … pane has no pixels yet" rather than writing a blank file.

An optional label is appended to the filename, so a script taking several produces readable names
rather than a column of timestamps. Labels are restricted to letters, digits, `-` and `_`, because
they become part of a filename; anything else is a parse error rather than a silently mangled name.

**Names are reproducible even though they are timestamps.** A replay run stamps from the scripted
clock, so a `pane` at t = 5 s is always `20260101T000005.000Z-waterfall.png` — on any machine, at
any hour.

`--capture <dir>` overrides `set run.capture`, the same precedence as `--dump` over `set run.dump`.
With neither, captures go to `capture.dir` from the config, which defaults to `./capture`.

A pane with no pixels yet — a script capturing before any spectrum has been processed — writes
nothing and says so. That is a legitimate outcome, but a missing file would otherwise look like a
broken directive.

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
