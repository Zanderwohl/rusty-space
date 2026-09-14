#!/usr/bin/env bash
# Every shader the client's materials name must be in the staged asset tree.
#
# This exists because the failure mode is invisible until it is not: a material whose WGSL is
# missing is a runtime 404 in a browser, long after the build said it succeeded. `em-render`
# ships material definitions and each host supplies the shader files, so the set is decided by
# which of its modules the client imports -- and that changes without anyone thinking about
# assets.
set -euo pipefail
cd "$(dirname "$0")/.."

ASSETS="${1:-crates/lc-client/assets}"

# Modules of em-render that lc-client actually imports, and the shaders those name.
modules=$(grep -rho 'em_render::[a-z_]*' crates/lc-client/src | sed 's/em_render:://' | sort -u)
wanted=$(
  for m in $modules; do
    [ -f "crates/em-render/src/$m.rs" ] && grep -ho 'shaders/[a-z_]*\.wgsl' "crates/em-render/src/$m.rs"
  done
  grep -rho 'shaders/[a-z_]*\.wgsl' crates/lc-client/src 2>/dev/null || true
)
wanted=$(printf '%s\n' "$wanted" | sort -u | grep . || true)

missing=0
for s in $wanted; do
  if [ -f "$ASSETS/$s" ]; then
    printf '    ok      %s\n' "$s"
  else
    printf '    MISSING %s\n' "$s" >&2
    missing=1
  fi
done
[ -n "$wanted" ] || { echo "    no shaders found -- the grep has stopped matching" >&2; exit 1; }
exit $missing
