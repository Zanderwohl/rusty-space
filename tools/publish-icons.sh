#!/usr/bin/env bash
# Publishes web/icons/ to `icons/` on the CDN, the originals and their .ico files both.
#
#   tools/publish-icons.sh [--force]
#
# **Nothing on a CDN is ever overwritten**, and the CDN serves everything `immutable`: an icon
# that changes needs a new file name, and the site pointed at it.
set -euo pipefail
cd "$(dirname "$0")/.."

CONTEXT=${CDN_DOCKER_CONTEXT:-rocinante}
CONTAINER=${CDN_CONTAINER:-lightcone-cdn}
DEST=/srv/cdn/icons
FORCE=
[ "${1:-}" = "--force" ] && FORCE=1

echo "==> publishing to $CONTEXT:$CONTAINER$DEST"
docker --context "$CONTEXT" exec "$CONTAINER" mkdir -p "$DEST"
for path in web/icons/*.svg web/icons/*.ico; do
  file=$(basename "$path")
  if docker --context "$CONTEXT" exec "$CONTAINER" test -f "$DEST/$file" 2>/dev/null; then
    if [ -z "$FORCE" ]; then
      echo "    skipped  $file (already published; CDN objects are immutable)"
      continue
    fi
    echo "    warning: overwriting $file" >&2
  fi
  docker --context "$CONTEXT" cp "$path" "$CONTAINER:$DEST/$file"
  echo "    sent     $file"
done
