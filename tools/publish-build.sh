#!/usr/bin/env bash
# Publishes a staged build to a CDN.
#
#   tools/publish-build.sh [build-id] [--force]
#
# This script is the seam. Today it copies into the development CDN's volume; the day there
# is a real bucket, the one command below becomes `aws s3 cp --recursive` with the headers the
# Caddyfile already declares, and nothing else here changes.
#
# **Nothing on a CDN is ever overwritten.** A new build is a new directory, which is what makes
# rollback a pointer change and means no cache ever needs purging. Publishing over an existing
# build id is refused unless you say so explicitly, and saying so is almost always a mistake.
set -euo pipefail
cd "$(dirname "$0")/.."

CONTEXT=${CDN_DOCKER_CONTEXT:-rocinante}
CONTAINER=${CDN_CONTAINER:-lightcone-cdn}
FORCE=

BUILD_ID=
while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    -*) echo "unknown option $1" >&2; exit 2 ;;
    *) BUILD_ID="$1"; shift ;;
  esac
done

# Default to the most recently staged build.
if [ -z "$BUILD_ID" ]; then
  BUILD_ID=$(ls -t target/web 2>/dev/null | head -1 || true)
fi
[ -n "$BUILD_ID" ] || { echo "no staged build; run tools/build-wasm.sh" >&2; exit 1; }

SRC="target/web/$BUILD_ID"
[ -d "$SRC" ] || { echo "$SRC does not exist" >&2; exit 1; }

# A dirty build id would pin players to a tree that exists on one laptop and nowhere else.
case "$BUILD_ID" in
  *-dirty)
    if [ -z "$FORCE" ]; then
      echo "build id '$BUILD_ID' is from a dirty tree." >&2
      echo "Commit first, or pass --force to publish something unreproducible." >&2
      exit 1
    fi
    echo "warning: publishing a dirty build" >&2
    ;;
esac

DEST="/srv/cdn/game/$BUILD_ID"

if docker --context "$CONTEXT" exec "$CONTAINER" test -d "$DEST" 2>/dev/null; then
  if [ -z "$FORCE" ]; then
    echo "$BUILD_ID is already published. CDN objects are immutable;" >&2
    echo "build a new one rather than overwriting this." >&2
    exit 1
  fi
  echo "warning: overwriting an existing build" >&2
  docker --context "$CONTEXT" exec "$CONTAINER" rm -rf "$DEST"
fi

echo "==> publishing $BUILD_ID to $CONTEXT:$CONTAINER$DEST"
docker --context "$CONTEXT" exec "$CONTAINER" mkdir -p "$DEST"
# `docker cp` streams a tar over the context's ssh connection, so this needs no rsync on the
# host and no shell in the image beyond what Caddy's alpine base already has.
docker --context "$CONTEXT" cp "$SRC/." "$CONTAINER:$DEST"

echo "==> published"
docker --context "$CONTEXT" exec "$CONTAINER" find "$DEST" -type f \
  | sed "s|$DEST/|    |" | sort
