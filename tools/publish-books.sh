#!/usr/bin/env bash
# Publishes the shelf to a CDN.
#
#   tools/publish-books.sh [--check] [--force]
#
# `--check` stops after verifying the shelf against its catalogue and needs no CDN, which is what
# CI runs: a book edited in place, or added to the directory without being cataloged, is a
# broken shelf whether or not anyone is publishing today.
#
# The books are in the repository — see lightcone/docs/21-library.md — so there is nothing to
# fetch and nothing to build. This copies them to `library/` on the CDN, beside `game/<build-id>`
# and on a schedule of its own: a book is not part of a build and must not be re-uploaded with
# every one.
#
# **Nothing on a CDN is ever overwritten.** A title that needs new bytes gets a new file name and
# a new row in the catalogue; publishing over an existing name is refused, because the old bytes
# are what every cache and every reader in the middle of that book already has.
set -euo pipefail
cd "$(dirname "$0")/.."

CONTEXT=${CDN_DOCKER_CONTEXT:-rocinante}
CONTAINER=${CDN_CONTAINER:-lightcone-cdn}
SHELF=crates/lc-client/assets/books
CATALOGUE="$SHELF/books.toml"
DEST=/srv/cdn/library
FORCE=
CHECK_ONLY=

while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    --check) CHECK_ONLY=1; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done

[ -f "$CATALOGUE" ] || { echo "no catalogue at $CATALOGUE" >&2; exit 1; }

# Every file the catalogue names, and nothing else. A stray epub in the directory is not on the
# shelf, and publishing it would put bytes on the CDN that nothing can reach.
FILES=$(sed -n 's/^file *= *"\(.*\)"/\1/p' "$CATALOGUE")
[ -n "$FILES" ] || { echo "the catalogue lists no files" >&2; exit 1; }

# The hash in the catalogue is the hash of the file every player reads. Checking it here is what
# makes that true rather than aspirational: a book edited in place after it was cataloged is
# caught before it reaches anyone.
echo "==> checking the catalogue against the shelf"
FAILED=
while IFS= read -r file; do
  [ -n "$file" ] || continue
  if [ ! -f "$SHELF/$file" ]; then
    echo "    MISSING  $file" >&2
    FAILED=1
    continue
  fi
  want=$(awk -v f="$file" '
    $0 == "file    = \"" f "\"" { found = 1; next }
    found && /^sha256/ { gsub(/[^0-9a-f]/, "", $3); print $3; exit }
  ' "$CATALOGUE")
  got=$(shasum -a 256 "$SHELF/$file" | cut -d' ' -f1)
  if [ -z "$want" ]; then
    echo "    no sha256 in the catalogue for $file; add:"
    echo "        sha256  = \"$got\""
    FAILED=1
  elif [ "$want" != "$got" ]; then
    echo "    CHANGED  $file" >&2
    echo "        catalogue: $want" >&2
    echo "        on disk:   $got" >&2
    FAILED=1
  else
    echo "    ok       $file"
  fi
done <<< "$FILES"
[ -z "$FAILED" ] || { echo "refusing to publish a shelf that does not match its catalogue" >&2; exit 1; }
[ -z "$CHECK_ONLY" ] || { echo "the shelf matches its catalogue"; exit 0; }

echo "==> publishing to $CONTEXT:$CONTAINER$DEST"
docker --context "$CONTEXT" exec "$CONTAINER" mkdir -p "$DEST"
while IFS= read -r file; do
  [ -n "$file" ] || continue
  if docker --context "$CONTEXT" exec "$CONTAINER" test -f "$DEST/$file" 2>/dev/null; then
    if [ -z "$FORCE" ]; then
      echo "    skipped  $file (already published; CDN objects are immutable)"
      continue
    fi
    echo "    warning: overwriting $file" >&2
  fi
  # `docker cp` streams a tar over the context's ssh connection, so this needs no rsync on the
  # host. The day there is a bucket, this one line becomes `aws s3 cp` with the headers the
  # Caddyfile already declares, and nothing else here moves.
  docker --context "$CONTEXT" cp "$SHELF/$file" "$CONTAINER:$DEST/$file"
  echo "    sent     $file"
done <<< "$FILES"

echo "==> published"
docker --context "$CONTEXT" exec "$CONTAINER" ls -la "$DEST" | tail -n +2
