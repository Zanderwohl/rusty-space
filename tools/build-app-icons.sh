#!/usr/bin/env bash
# Renders the desktop app icons into crates/lc-icon/icons/<name>/: transparent PNGs at every
# size, a macOS .icns and a Windows .ico.
#
#   tools/build-app-icons.sh
#
#   galaxy-30, galaxy-45, galaxy-60   the site's micro-galaxy, tilted that many degrees above
#                                     its plane. Galaxy Forge uses galaxy-30.
#   lightcone-green                   web/icons/lightcone-green.svg. The desktop client uses it.
#
# Headless Chrome draws both: the galaxy through tools/app-icon/render.html, and the SVG
# because ImageMagick drops its gradients (see tools/build-icons.sh). The outputs are
# committed, so building either app needs neither tool.
set -euo pipefail
cd "$(dirname "$0")/.."

CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
SIZES="16 24 32 48 64 128 256 512 1024"
# The fraction of a turn the galaxy is drawn at.
TURN=0.125
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

shoot() { # url size out.png
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --default-background-color=00000000 --window-size="$2,$2" --allow-file-access-from-files \
    --screenshot="$3" "$1" >/dev/null 2>&1
}

package() { # dir
  local out=$1 iconset="$WORK/icon.iconset"
  rm -rf "$iconset" && mkdir "$iconset"
  # iconutil names each size by its point size and scale.
  for n in 16 32 128 256 512; do
    cp "$out/icon-$n.png" "$iconset/icon_${n}x${n}.png"
    cp "$out/icon-$((n * 2)).png" "$iconset/icon_${n}x${n}@2x.png"
  done
  iconutil --convert icns --output "$out/icon.icns" "$iconset"

  # PNG-in-ICO, as in tools/build-icons.sh. A size byte of 0 means 256.
  python3 - "$out/icon.ico" "$out"/icon-{16,24,32,48,64,128,256}.png <<'PY'
import struct, sys
out, *paths = sys.argv[1:]
images = [open(p, "rb").read() for p in paths]
offset = 6 + 16 * len(images)
entries = b""
for data in images:
    w, h = struct.unpack(">II", data[16:24])
    entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
open(out, "wb").write(struct.pack("<HHH", 0, 1, len(images)) + entries + b"".join(images))
PY
  echo "    built    $out"
}

# The galaxy's textures are raster, so it is drawn once large and scaled down.
for elevation in 30 45 60; do
  out=crates/lc-icon/icons/galaxy-$elevation
  mkdir -p "$out"
  shoot "file://$PWD/tools/app-icon/render.html?size=1024&t=$TURN&elevation=$elevation" 1024 \
    "$WORK/master.png"
  for n in $SIZES; do
    magick "$WORK/master.png" -filter Lanczos -resize "${n}x${n}" -strip "PNG32:$out/icon-$n.png"
  done
  package "$out"
done

# The SVG is drawn at each size, so small sizes get the browser's hinting rather than a
# downscale.
out=crates/lc-icon/icons/lightcone-green
mkdir -p "$out"
for n in $SIZES; do
  printf '<!doctype html><body style="margin:0"><img src="file://%s" width=%s height=%s style="display:block">' \
    "$PWD/web/icons/lightcone-green.svg" "$n" "$n" > "$WORK/page.html"
  shoot "file://$WORK/page.html" "$n" "$WORK/shot.png"
  magick "$WORK/shot.png" -strip "PNG32:$out/icon-$n.png"
done
package "$out"
