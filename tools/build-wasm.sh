#!/usr/bin/env bash
# Builds the browser client and stages it as one immutable directory.
#
# Output: target/web/<build-id>/ containing the wasm, its glue, the assets it loads, and a
# manifest naming them. Nothing in here is ever edited in place -- a new build is a new
# directory, which is what lets the CDN serve it with a one-year immutable header.
#
#   tools/build-wasm.sh [--limit N] [--skip-opt]
#
# See lightcone/docs/14-hosting.md.
set -euo pipefail

cd "$(dirname "$0")/.."

BIN=lightcone_web
PROFILE=wasm-release
TARGET=wasm32-unknown-unknown
# A little above the client's own SKY_LIMIT, so raising that does not silently shorten the sky.
SKY_LIMIT=8000
SKIP_OPT=

while [ $# -gt 0 ]; do
  case "$1" in
    --limit) SKY_LIMIT="$2"; shift 2 ;;
    --skip-opt) SKIP_OPT=1; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done

BUILD_ID="$(git rev-parse --short HEAD)$(git diff --quiet || echo -dirty)"
OUT="target/web/$BUILD_ID"
CATALOGUE=assets/catalogs/hygdata_v42.csv

for tool in wasm-bindgen wasm-opt; do
  command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 1; }
done

echo "==> build $BUILD_ID"
rm -rf "$OUT"; mkdir -p "$OUT/assets"

# The sky, packed. The browser never sees a CSV: there is no filesystem to read one from and
# no reason to send 34 MB when 0.3 will do.
echo "==> sky chunk"
mkdir -p "$OUT/assets/sky"
cargo run -q -p lc-world --bin skypack -- "$CATALOGUE" "$OUT/assets/sky/hyg-v42.lcsky" \
  --limit "$SKY_LIMIT"

# `--no-default-features` turns off `hyg`, which is what keeps the CSV reader -- and the only
# `std::fs` call in the client's tree -- out of the browser binary entirely.
echo "==> cargo build"
cargo build -q -p lc-client --bin "$BIN" --target "$TARGET" \
  --profile "$PROFILE" --no-default-features

echo "==> wasm-bindgen"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" \
  "target/$TARGET/$PROFILE/$BIN.wasm"

if [ -z "$SKIP_OPT" ]; then
  echo "==> wasm-opt -Oz"
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-reference-types \
    "$OUT/${BIN}_bg.wasm" -o "$OUT/${BIN}_bg.opt.wasm"
  mv "$OUT/${BIN}_bg.opt.wasm" "$OUT/${BIN}_bg.wasm"
fi

# Shaders. The client's own asset root already holds every one its materials name; see the
# check below, which is what stops that quietly ceasing to be true.
echo "==> assets"
cp -R crates/lc-client/assets/shaders "$OUT/assets/shaders"
tools/check-shaders.sh "$OUT/assets"

WASM_BYTES=$(wc -c < "$OUT/${BIN}_bg.wasm" | tr -d ' ')
JS_BYTES=$(wc -c < "$OUT/$BIN.js" | tr -d ' ')
SKY_BYTES=$(wc -c < "$OUT/assets/sky/hyg-v42.lcsky" | tr -d ' ')

cat > "$OUT/manifest.json" <<JSON
{
  "build_id": "$BUILD_ID",
  "engine": "bevy-0.17",
  "requires": ["webgpu"],
  "entry": "$BIN.js",
  "wasm": "${BIN}_bg.wasm",
  "asset_base": "assets",
  "sky": "assets/sky/hyg-v42.lcsky",
  "bytes": { "wasm": $WASM_BYTES, "js": $JS_BYTES, "sky": $SKY_BYTES }
}
JSON

cp tools/wasm-index.html "$OUT/index.html"

# Pre-compress, beside the original. This is what the real upload does -- object storage will
# not compress for you, and forgetting it quadruples the download. A CDN serves `foo.wasm.br`
# with `Content-Encoding: br` while keeping the original content type.
#
# Gzip as well as brotli, and not for old browsers: **Chrome only advertises `br` on a secure
# origin**. The development CDN is plain HTTP, so a real browser asks it for `gzip, deflate`
# and would silently take the whole 27 MB uncompressed if gzip were absent -- which is a
# compression path that only ever gets exercised in production, where nobody is watching it.
echo "==> pre-compress"
compressible() {
  find "$OUT" -type f \( -name '*.wasm' -o -name '*.js' -o -name '*.wgsl' \
    -o -name '*.lcsky' -o -name '*.json' -o -name '*.html' \) -print0
}
if command -v brotli >/dev/null; then
  compressible | while IFS= read -r -d '' f; do brotli -q 11 -f -k "$f" -o "$f.br"; done
else
  echo "    brotli not installed -- no .br, and production would serve uncompressed" >&2
fi
compressible | while IFS= read -r -d '' f; do gzip -9 -f -k -c "$f" > "$f.gz"; done

echo
echo "==> $OUT"
for f in "$OUT/${BIN}_bg.wasm" "$OUT/$BIN.js" "$OUT/assets/sky/hyg-v42.lcsky"; do
  raw=$(wc -c < "$f" | tr -d ' ')
  if [ -f "$f.br" ]; then br=$(wc -c < "$f.br" | tr -d ' '); else br=0; fi
  printf '    %-28s %8.2f MB raw  %8.2f MB brotli\n' \
    "$(basename "$f")" "$(echo "$raw/1000000" | bc -l)" "$(echo "$br/1000000" | bc -l)"
done
