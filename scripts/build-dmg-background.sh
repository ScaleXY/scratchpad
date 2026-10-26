#!/usr/bin/env bash
# Renders packaging/macos/dmg-background.svg into dmg-background.tiff (1x and 2x for Retina),
# the background of the macOS .dmg window. Needs macOS (its fonts and tiffutil) and Google Chrome.
#   scripts/build-dmg-background.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SVG="$ROOT/packaging/macos/dmg-background.svg"
OUT="$ROOT/packaging/macos/dmg-background.tiff"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cat > "$WORK/page.html" <<EOF
<!doctype html><html><body style="margin:0;overflow:hidden">$(cat "$SVG")</body></html>
EOF
for scale in 1 2; do
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor="$scale" \
    --window-size=660,420 --screenshot="$WORK/background-$scale.png" "file://$WORK/page.html" 2>/dev/null
done
tiffutil -cathidpicheck "$WORK/background-1.png" "$WORK/background-2.png" -out "$OUT" 2>/dev/null
[[ -n "${PREVIEW:-}" ]] && cp "$WORK/background-2.png" "$PREVIEW"
echo "Wrote $OUT"
