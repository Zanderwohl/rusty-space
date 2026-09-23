#!/usr/bin/env python3
"""Print the public API surface of one or more crates, and flag oversized modules.

Usage: tools/api_surface.py crates/lc-spacetime [crates/em-spectra ...]

Strips `#[cfg(test)]` modules, so the line counts are code, not tests.
"""
import re
import sys
from pathlib import Path

LIMIT = 1000

# A module may raise its own cap by saying so, in its own module doc, with a reason:
#
#     //! Line limit: 2000. <why this module is the exception>
#
# Read from the file rather than listed here, so the number and the justification cannot drift
# apart and a reviewer meets the reason where the code is. The reason is required: a bare number
# raises the cap while saying nothing, which is the one thing the rule exists to stop.
RAISED = re.compile(r"^//!\s*Line limit:\s*(?P<limit>\d+)\s*[.:-]\s*(?P<why>\S.*)$")

# A raise with no reason after the number. Reported rather than honored.
BARE_RAISE = re.compile(r"^//!\s*Line limit:\s*\d+\s*[.:-]?\s*$")

ITEM = re.compile(
    r"^(?P<indent>\s*)(?P<sig>pub(?:\s*\([^)]*\))?\s+"
    r"(?:const\s+fn|async\s+fn|unsafe\s+fn|fn|struct|enum|trait|type|const|static|mod|union)\b.*)$"
)
IMPL = re.compile(r"^(?P<indent>\s*)(?P<sig>impl\b.*?)(?:\s*\{\s*)?$")


def strip_tests(lines):
    """Remove `#[cfg(test)] mod ... { ... }` blocks by brace matching."""
    out, i, n = [], 0, len(lines)
    while i < n:
        if lines[i].lstrip().startswith("#[cfg(test)]"):
            j = i
            while j < n and "{" not in lines[j]:
                j += 1
            if j >= n:
                break
            depth = 0
            while j < n:
                depth += lines[j].count("{") - lines[j].count("}")
                j += 1
                if depth <= 0:
                    break
            i = j
            continue
        out.append(lines[i])
        i += 1
    return out


def tidy(sig):
    sig = sig.rstrip()
    for suffix in (" {", "{"):
        if sig.endswith(suffix):
            sig = sig[: -len(suffix)].rstrip()
    return sig


def continuation(lines, start):
    """Join a signature split over several lines, up to `{`, `;` or `)`."""
    sig = lines[start].strip()
    k = start
    while k + 1 < len(lines) and not re.search(r"[;{]\s*$|\)\s*(->\s*[^{;]+)?\s*$", sig):
        k += 1
        sig += " " + lines[k].strip()
        if k - start > 12:
            break
    return re.sub(r"\s+", " ", sig), k


def main(roots):
    total = 0
    oversize = []
    unjustified = []
    for root in roots:
        src = Path(root) / "src"
        if not src.is_dir():
            print(f"!! {root}: no src/", file=sys.stderr)
            continue
        print(f"\n{'=' * 78}\n{root}\n{'=' * 78}")
        for path in sorted(src.rglob("*.rs")):
            raw = path.read_text().splitlines()
            code = strip_tests(raw)
            loc = len(code)
            total += loc
            limit = LIMIT
            for line in raw[:40]:
                stripped = line.strip()
                raised = RAISED.match(stripped)
                if raised:
                    limit = int(raised.group("limit"))
                    break
                if BARE_RAISE.match(stripped):
                    unjustified.append(path)
                    break
            flag = f"  ** OVER LIMIT **" if loc > limit else ""
            if limit != LIMIT:
                flag += f"  (own limit {limit})"
            if loc > limit:
                oversize.append((path, loc))
            rel = path.relative_to(Path(root) / "src")
            print(f"\n-- {rel}  ({loc} loc, {len(raw)} with tests){flag}")
            i = 0
            trait_depth = None
            depth_now = 0
            while i < len(code):
                line = code[i]
                if trait_depth is not None:
                    depth_now += line.count("{") - line.count("}")
                    if depth_now <= trait_depth:
                        trait_depth = None
                m = ITEM.match(line) or IMPL.match(line)
                # Trait methods are not `pub`, but they are the trait's surface.
                if not m and trait_depth is not None:
                    m = re.match(r"^(?P<indent>\s+)(?P<sig>fn\s+.*)$", line)
                if m and not line.lstrip().startswith("//"):
                    sig, i = continuation(code, i)
                    indent = len(m.group("indent")) // 4
                    print(f"   {'  ' * indent}{tidy(sig)}")
                    if re.search(r"\btrait\s+\w+", sig) and not sig.rstrip().endswith(";"):
                        trait_depth = 0
                        # The opening brace is already consumed by `sig`.
                        depth_now = sig.count("{") - sig.count("}")
                i += 1
    print(f"\n{'=' * 78}")
    print(f"{total} lines of code across {len(roots)} crate(s), limit {LIMIT} per file")
    if unjustified:
        print("RAISED WITH NO REASON GIVEN:")
        for p in unjustified:
            print(f"  {p}")
    if oversize:
        print("OVER LIMIT:")
        for p, n in oversize:
            print(f"  {p}: {n}")
    if oversize or unjustified:
        return 1
    print("all modules within limit")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:] or ["crates/lc-spacetime"]))
