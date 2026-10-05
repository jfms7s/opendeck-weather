#!/usr/bin/env sh
# Rasterizes the action-list icon sources (assets/icon-src/*.svg, generated
# from src/glyphs.rs by `cargo test -- --ignored write_icon_sources`) into
# assets/icons/<name>.png (144 px) and <name>@2x.png (288 px).
# Needs rsvg-convert (librsvg), or ImageMagick built with librsvg.
set -eu
cd "$(dirname "$0")/.."

for svg in assets/icon-src/*.svg; do
	name=$(basename "$svg" .svg)
	for size in 144 288; do
		out="assets/icons/$name.png"
		[ "$size" = 288 ] && out="assets/icons/$name@2x.png"
		if command -v rsvg-convert >/dev/null 2>&1; then
			rsvg-convert -w "$size" -h "$size" -o "$out" "$svg"
		else
			# The sources are 30 units wide: render at the matching density.
			magick -background none -density $((72 * size / 30)) "RSVG:$svg" \
				-resize "${size}x${size}" "PNG64:$out"
		fi
		echo "wrote $out"
	done
done
