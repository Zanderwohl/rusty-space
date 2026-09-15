#!/usr/bin/env bash
# Register, promote, yank and list game builds.
#
#   tools/release.sh list
#   tools/release.sh register [build-id]        # defaults to the newest staged build
#   tools/release.sh promote  <build-id> [chan] # chan defaults to `stable`
#   tools/release.sh yank     <build-id>
#   tools/release.sh unyank   <build-id>
#   tools/release.sh ship     [build-id]        # build, publish, register, promote
#
# Registering says a build exists. Promoting says players get it. They are separate because
# conflating them means eventually promoting something nobody looked at -- so `ship` exists for
# development, where you are the only player, and is the one command that does both.
set -euo pipefail
cd "$(dirname "$0")/.."

SITE=${LC_SITE:-http://localhost:3100}
CDN=${LC_CDN:-http://rocinante.local:3101}
TOKEN=${RELEASE_TOKEN:-}

[ -n "$TOKEN" ] || { echo "RELEASE_TOKEN is not set" >&2; exit 1; }

api() {
  local method=$1 path=$2 body=${3:-}
  local args=(-sS -X "$method" -H "x-release-token: $TOKEN")
  [ -n "$body" ] && args+=(-H 'content-type: application/json' -d "$body")
  curl "${args[@]}" -w '\n%{http_code}' "$SITE$path"
}

# Prints the body and fails on a non-2xx, so a shell `&&` chain stops where it should.
call() {
  local out status
  out=$(api "$@")
  status=${out##*$'\n'}
  body=${out%$'\n'*}
  if [ "$status" -ge 300 ]; then
    echo "$body" >&2
    echo "  ($status from $2)" >&2
    return 1
  fi
  printf '%s\n' "$body"
}

newest_staged() { ls -t target/web 2>/dev/null | head -1; }

pretty() { python3 -m json.tool 2>/dev/null || cat; }

cmd=${1:-list}
shift || true

case "$cmd" in
  list)
    call GET /internal/releases | python3 -c '
import json, sys
d = json.load(sys.stdin)
chan = {c["build_id"]: c["name"] for c in d["channels"]}
if not d["releases"]:
    print("no builds registered")
for r in d["releases"]:
    marks = []
    if r["build_id"] in chan: marks.append(chan[r["build_id"]].upper())
    if r["yanked"]: marks.append("yanked")
    size = f'"'"'{r["wasm_bytes"]/1e6:.1f} MB'"'"' if r.get("wasm_bytes") else ""
    print(f'"'"'{r["build_id"]:<12} {r["published_at"][:19]:<20} {size:>9}  {" ".join(marks)}'"'"')
'
    ;;

  register)
    build=${1:-$(newest_staged)}
    [ -n "$build" ] || { echo "no build id and nothing staged" >&2; exit 1; }
    # A build row carries its own CDN, and `LC_CDN` defaults to the development one. Register
    # a plain-http CDN against an https site and every browser blocks the load as mixed
    # content -- which surfaces as "the site is pointing at a build that is not on the CDN",
    # naming a URL that is perfectly reachable by hand. Refuse it here instead.
    case "$SITE:$CDN" in
      https://*:http://*)
        echo "refusing: $SITE is https and LC_CDN is $CDN" >&2
        echo "a browser will not load http assets into an https page. Set LC_CDN to the" >&2
        echo "https CDN this site serves, e.g. LC_CDN=https://cdn.lc.zanderlowry.com" >&2
        exit 1
        ;;
    esac
    bytes=$(wc -c < "target/web/$build/lightcone_web_bg.wasm" 2>/dev/null | tr -d ' ' || echo null)
    call POST /internal/release \
      "{\"build_id\":\"$build\",\"cdn_base\":\"$CDN\",\"wasm_bytes\":${bytes:-null}}" >/dev/null
    echo "registered $build"
    ;;

  promote)
    build=${1:?build id}; channel=${2:-stable}
    call POST "/internal/channel/$channel" "{\"build_id\":\"$build\"}" >/dev/null
    echo "$channel -> $build"
    ;;

  yank|unyank)
    build=${1:?build id}
    [ "$cmd" = yank ] && y=true || y=false
    call POST "/internal/release/$build/yank" "{\"yanked\":$y}" >/dev/null
    echo "$cmd $build"
    ;;

  ship)
    build=${1:-}
    if [ -z "$build" ]; then
      tools/build-wasm.sh
      build=$(newest_staged)
    fi
    tools/publish-build.sh "$build"
    "$0" register "$build"
    "$0" promote "$build"
    echo
    "$0" list
    ;;

  *) echo "unknown command $cmd" >&2; exit 2 ;;
esac
