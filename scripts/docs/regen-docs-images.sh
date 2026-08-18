#!/bin/sh
# Copyright (c) 2026 G & R Associates LLC
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Regenerate docs/images/*.png from the headless replay recipes beside this
# script.
#
#   scripts/docs/regen-docs-images.sh                  # all of them, in place
#   scripts/docs/regen-docs-images.sh source-am-dsb    # just one
#   scripts/docs/regen-docs-images.sh --out /tmp/shots # elsewhere, to compare
#
# One recipe per image, matched by name and nothing else:
#
#   scripts/docs/<name>.txt   the script; its `still` label is <name>
#   docs/images/<name>.png    what lands
#
# So adding an image to the README is adding one file here, with no edit to this
# script.  A recipe states its own `set run.size` and `set run.scale`, which is
# what keeps every image 2400x1656 at 72 dpi; a run that produces some other size
# is reported rather than silently committed.  Anything else a recipe needs — a
# C/N, a burst length — it states with `set` too, so there is nothing beside it
# to remember.
#
# The captures are reproducible run to run on one machine, but NOT across
# architectures — rustfft dispatches AVX on x86-64 and Neon on AArch64, so the
# spectrum differs in its last bits and every pixel downstream inherits it.
# Regenerating on a different machine will therefore rewrite every file even
# when nothing has changed.  See docs/headless.md.

set -eu

expect=2400x1656

usage() {
	cat <<'EOF'
usage: regen-docs-images.sh [options] [name ...]

  --out DIR     write the PNGs here instead of docs/images
  --bin PATH    use this binary instead of building target/release
  --no-build    skip cargo build; use the existing target/release binary
  -h, --help    this

With no names, every recipe in scripts/docs/ is run.
EOF
}

unset CDPATH # so `cd` prints nothing and lands where it was told
here=$(cd -- "$(dirname -- "$0")" && pwd)
root=$(cd -- "$here/../.." && pwd)
recipes=$here
out=$root/docs/images
bin=
build=1
names=

while [ $# -gt 0 ]; do
	case $1 in
	-h | --help)
		usage
		exit 0
		;;
	--out)
		out=${2:?--out needs a directory}
		shift 2
		;;
	--bin)
		bin=${2:?--bin needs a path}
		shift 2
		;;
	--no-build)
		build=0
		shift
		;;
	-*)
		echo "regen-docs-images: unknown option: $1" >&2
		usage >&2
		exit 2
		;;
	*)
		names="$names $1"
		shift
		;;
	esac
done

if [ -z "$bin" ]; then
	bin=$root/target/release/orion-sdr-view
	if [ "$build" -eq 1 ]; then
		echo "regen-docs-images: cargo build --release"
		(cd "$root" && cargo build --release)
	fi
fi
if [ ! -x "$bin" ]; then
	echo "regen-docs-images: no binary at $bin (drop --no-build, or pass --bin)" >&2
	exit 1
fi
mkdir -p "$out"

# Resolve the names to run.  A name may be given bare, or as the recipe or image
# filename, so tab completion on either directory does the right thing.
scripts=
if [ -n "$names" ]; then
	for n in $names; do
		n=$(basename "$n")
		n=${n%.txt}
		n=${n%.png}
		if [ ! -f "$recipes/$n.txt" ]; then
			echo "regen-docs-images: no recipe $recipes/$n.txt" >&2
			exit 2
		fi
		scripts="$scripts $recipes/$n.txt"
	done
else
	for f in "$recipes"/*.txt; do
		scripts="$scripts $f"
	done
fi

# PNG width and height, read from the IHDR chunk: bytes 16..23 of the file, two
# big-endian u32s.  `sips` would do it in one call and only on macOS.
png_size() {
	od -An -tu1 -j16 -N8 "$1" | awk 'NR == 1 {
		printf "%dx%d", $1*16777216 + $2*65536 + $3*256 + $4,
		                $5*16777216 + $6*65536 + $7*256 + $8
	}'
}

status=0
for script in $scripts; do
	name=$(basename "$script" .txt)
	tmp=$(mktemp -d "${TMPDIR:-/tmp}/orion-docs-images.XXXXXX")

	# The driver narrates each run on stderr; hold it back and show it only if
	# the run fails, so a clean regeneration prints one line per image.
	if ! "$bin" --headless --script "$script" --capture "$tmp" \
		>/dev/null 2>"$tmp/log"; then
		cat "$tmp/log" >&2
		echo "regen-docs-images: $name: the run failed" >&2
		rm -rf "$tmp"
		status=1
		continue
	fi

	# The still is named <scripted timestamp>-<label>.png.  Match on the label
	# rather than the stamp, so retiming a capture does not touch this script.
	png=
	for f in "$tmp"/*-"$name".png; do
		[ -f "$f" ] && png=$f
	done
	if [ -z "$png" ]; then
		echo "regen-docs-images: $name: the run wrote no still labelled $name" >&2
		rm -rf "$tmp"
		status=1
		continue
	fi

	size=$(png_size "$png")
	mv "$png" "$out/$name.png"
	rm -rf "$tmp"

	if [ "$size" = "$expect" ]; then
		echo "  $out/$name.png  $size"
	else
		echo "  $out/$name.png  $size" >&2
		echo "regen-docs-images: $name: $size, expected $expect — check its size/scale" >&2
		status=1
	fi
done

exit $status
