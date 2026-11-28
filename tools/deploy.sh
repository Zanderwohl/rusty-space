#!/usr/bin/env bash
# Builds the stack's images at this commit on rocinante and brings the stack up there.
#
#   tools/deploy.sh [--force] [--no-build] [service...]
#
# With services named (db cdn proxy identity web admin shard), only those are built and
# recreated, and nothing they depend on is touched. Without, all of them.
#
# Builds run from this checkout on the rocinante docker context. `compose up` runs on the host
# over ssh, because compose reads `--env-file` where it is invoked and the settings file,
# ~/.config/lightcone/lightcone.env, never leaves the host. See lightcone/docs/15-runbook.md.
set -euo pipefail
cd "$(dirname "$0")/.."

HOST=${DEPLOY_HOST:-zandy@rocinante.alexanderlowry.com}
CONTEXT=${DEPLOY_DOCKER_CONTEXT:-rocinante}
DIR='~/.config/lightcone'
FORCE=
BUILD=1
SERVICES=()

while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    --no-build) BUILD=; shift ;;
    -*) echo "unknown option $1" >&2; exit 2 ;;
    *) SERVICES+=("$1"); shift ;;
  esac
done

TAG=$(git rev-parse --short HEAD)
if ! git diff --quiet HEAD; then
  [ -n "$FORCE" ] || { echo "the tree is dirty; commit first, or pass --force to deploy $TAG with changes it does not name" >&2; exit 1; }
  echo "warning: deploying a dirty tree as $TAG" >&2
fi
export TAG

# The database runs a stock image and has nothing to build.
BUILT=()
for s in ${SERVICES[@]+"${SERVICES[@]}"}; do [ "$s" = db ] || BUILT+=("$s"); done

if [ -n "$BUILD" ] && { [ ${#SERVICES[@]} -eq 0 ] || [ ${#BUILT[@]} -gt 0 ]; }; then
  echo "==> building $TAG on $CONTEXT"
  docker --context "$CONTEXT" compose -f deploy/compose.build.yaml build ${BUILT[@]+"${BUILT[@]}"}
fi

echo "==> copying compose.yaml to $HOST:$DIR"
scp -q deploy/compose.yaml "$HOST:.config/lightcone/compose.yaml"

UP="up -d"
[ ${#SERVICES[@]} -eq 0 ] || UP="up -d --no-deps ${SERVICES[*]}"
echo "==> $UP at $TAG"
ssh "$HOST" "export TAG=$TAG && cd $DIR && docker compose -f compose.yaml --env-file lightcone.env $UP && docker compose -f compose.yaml --env-file lightcone.env ps --format '{{.Name}}\t{{.Image}}\t{{.Status}}'"
