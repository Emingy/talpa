#!/usr/bin/env bash
# Converts assets/logo.svg → assets/AppIcon.icns
# Requires: librsvg  (brew install librsvg)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SVG="$ROOT/assets/logo.svg"
ICONSET="$ROOT/assets/AppIcon.iconset"
ICNS="$ROOT/assets/AppIcon.icns"

if ! command -v rsvg-convert &>/dev/null; then
    echo "rsvg-convert not found. Install with: brew install librsvg" >&2
    exit 1
fi

mkdir -p "$ICONSET"

for size in 16 32 128 256 512; do
    rsvg-convert -w $size        -h $size        "$SVG" -o "$ICONSET/icon_${size}x${size}.png"
    rsvg-convert -w $((size*2))  -h $((size*2))  "$SVG" -o "$ICONSET/icon_${size}x${size}@2x.png"
done

iconutil -c icns "$ICONSET" -o "$ICNS"
rm -rf "$ICONSET"
echo "Created $ICNS"
