#!/usr/bin/env bash
# Rasterizes every SVG in web/icons/ into a multi-size .ico beside it.
#
#   tools/build-icons.sh
#
# Rendered by headless Chrome, because ImageMagick's own SVG renderer drops gradients
# declared in user space under a transform, which is how these icons are drawn. The .ico
# files are committed, so publishing needs no browser.
set -euo pipefail
cd "$(dirname "$0")/.."

CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
SIZES="16 32 48 64"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

for svg in web/icons/*.svg; do
  name=$(basename "$svg" .svg)
  for n in $SIZES; do
    printf '<!doctype html><body style="margin:0"><img src="file://%s" width=%s height=%s style="display:block">' \
      "$PWD/$svg" "$n" "$n" > "$WORK/page.html"
    "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
      --default-background-color=00000000 --window-size="$n,$n" --allow-file-access-from-files \
      --screenshot="$WORK/$name-$n.png" "file://$WORK/page.html" >/dev/null 2>&1
  done
  # PNG-in-ICO: every browser that renders a favicon reads it, and it keeps the alpha exact.
  python3 - "$WORK" "$name" "web/icons/$name.ico" $SIZES <<'PY'
import struct, sys
work, name, out, *sizes = sys.argv[1:]
images = [open(f"{work}/{name}-{n}.png", "rb").read() for n in sizes]
offset = 6 + 16 * len(images)
header = struct.pack("<HHH", 0, 1, len(images))
entries = b""
for n, data in zip(map(int, sizes), images):
    entries += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
open(out, "wb").write(header + entries + b"".join(images))
PY
  echo "    built    web/icons/$name.ico"
done
