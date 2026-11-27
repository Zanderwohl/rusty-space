#!/usr/bin/env bash
# Publishes a galaxy that `lc-galaxy publish` wrote to a directory onto a CDN, under
# galaxy/<name>/.
#
#   tools/publish-galaxy.sh <dir> [--name <name>] [--dry-run]
#
# Copies chunks/<hash>.lcg and <hash>.galaxy, chunks first, so a manifest on the CDN never names
# a chunk that is not there yet. `manifest.galaxy` stays local: its name does not change between
# publishes, and nothing on a CDN is overwritten.
#
# A file already on the CDN with the same bytes is skipped. One with different bytes under the
# same name is refused before anything is copied: a name is a content hash, so that is a damaged
# file or a hash collision, and either needs a person.
#
# Like tools/publish-build.sh, this copies into the development CDN's volume; with a bucket the
# copy becomes an upload with the Caddyfile's headers.
set -euo pipefail
cd "$(dirname "$0")/.."

CONTEXT=${CDN_DOCKER_CONTEXT:-rocinante}
CONTAINER=${CDN_CONTAINER:-lightcone-cdn}
SRC=
NAME=
DRY=

while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="$2"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    -*) echo "unknown option $1" >&2; exit 2 ;;
    *) SRC="$1"; shift ;;
  esac
done

[ -n "$SRC" ] || { echo "usage: tools/publish-galaxy.sh <dir> [--name <name>] [--dry-run]" >&2; exit 2; }
[ -f "$SRC/manifest.galaxy" ] || { echo "$SRC/manifest.galaxy does not exist; run lc-galaxy publish" >&2; exit 1; }

if [ -z "$NAME" ]; then
  NAME=$(sed -n 's/^ *name: *"\(.*\)",$/\1/p' "$SRC/manifest.galaxy" | head -1)
fi
case "$NAME" in
  ''|*/*|.*) echo "galaxy name '$NAME' is not a plain directory name; pass --name" >&2; exit 1 ;;
esac

# The hashed copy of manifest.galaxy is the one published.
MANIFEST=
for f in "$SRC"/*.galaxy; do
  [ "$(basename "$f")" = manifest.galaxy ] && continue
  cmp -s "$f" "$SRC/manifest.galaxy" && MANIFEST=$(basename "$f")
done
[ -n "$MANIFEST" ] || { echo "no <hash>.galaxy in $SRC matches manifest.galaxy; republish" >&2; exit 1; }

FILES=$( (cd "$SRC" && ls chunks/*.lcg; echo "$MANIFEST") )
for f in $FILES; do
  [ -f "$SRC/$f" ] || { echo "$SRC/$f is missing" >&2; exit 1; }
done

DEST="/srv/cdn/galaxy/$NAME"
REMOTE=$(docker --context "$CONTEXT" exec "$CONTAINER" sh -c \
  "cd '$DEST' 2>/dev/null && sha256sum chunks/*.lcg *.galaxy 2>/dev/null" || true)

MISSING=
CLASH=
for f in $FILES; do
  local_sum=$(shasum -a 256 "$SRC/$f" | cut -d' ' -f1)
  remote_sum=$(printf '%s\n' "$REMOTE" | awk -v f="$f" '$2 == f { print $1 }')
  if [ -z "$remote_sum" ]; then
    MISSING="$MISSING $f"
  elif [ "$remote_sum" != "$local_sum" ]; then
    CLASH="$CLASH $f"
  fi
done

if [ -n "$CLASH" ]; then
  echo "already on the CDN with different bytes:" >&2
  for f in $CLASH; do echo "    $f" >&2; done
  exit 1
fi

URL="galaxy/$NAME/$MANIFEST"
if [ -z "$MISSING" ]; then
  echo "==> nothing to copy; $URL is already published"
  exit 0
fi

echo "==> copying to $CONTEXT:$CONTAINER$DEST"
for f in $MISSING; do echo "    $f"; done
if [ -n "$DRY" ]; then
  echo "==> dry run; nothing copied"
  exit 0
fi

# Stage only the missing chunks, then the manifest, each as its own `docker cp`.
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/chunks"
for f in $MISSING; do
  case "$f" in chunks/*) cp "$SRC/$f" "$STAGE/$f" ;; esac
done
docker --context "$CONTEXT" exec "$CONTAINER" mkdir -p "$DEST/chunks"
if [ -n "$(ls "$STAGE/chunks")" ]; then
  docker --context "$CONTEXT" cp "$STAGE/chunks/." "$CONTAINER:$DEST/chunks"
fi
case " $MISSING " in
  *" $MANIFEST "*) docker --context "$CONTEXT" cp "$SRC/$MANIFEST" "$CONTAINER:$DEST/$MANIFEST" ;;
esac

echo "==> published $URL"
echo "    the shard's --sky: http://$CONTAINER:3101/$URL"
