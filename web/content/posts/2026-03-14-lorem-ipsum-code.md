+++
title = "Excepteur sint occaecat: code blocks in four languages"
summary = "Rust, TOML, SQL and shell, plus a deliberately over-wide line, to check highlighting and horizontal overflow."
tags = ["placeholder", "code"]
updated = "2026-04-02"
+++

Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. This post exists to look at
code, so most of it is code.

## Rust

Ut enim ad minim veniam, quis nostrud exercitation:

```rust
/// Lorem ipsum dolor sit amet, consectetur adipiscing elit.
pub fn retarded_time(observer: Coord, source: &dyn Worldline) -> SmallVec<[Instant; 2]> {
    let mut roots = SmallVec::new();
    for candidate in source.crossings(observer.t) {
        if candidate.interval2(observer) == Separation::Lightlike {
            roots.push(candidate.t);
        }
    }
    roots
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Separation {
    Timelike,
    Lightlike,
    Spacelike,
}
```

A line that is far too long to fit in the column, deliberately, so that the horizontal scroll behaviour of a code block can be checked rather than assumed to work:

```rust
let observed = instrument.measure(&system, observer_worldline, Instant::from_julian_days(2451545.0), Band::V, Exposure::seconds(600.0));
```

## TOML

Duis aute irure dolor in reprehenderit:

```toml
[profile.wasm-release]
inherits      = "release"
opt-level     = "s"
lto           = "fat"
codegen-units = 1
panic         = "abort"
```

## SQL

Excepteur sint occaecat cupidatat non proident:

```sql
select observer_id, event_id, arrive_t
from deliveries
where observer_id = $1
  and arrive_t > $2
  and arrive_t <= $3
order by arrive_t;
```

## Shell

Sunt in culpa qui officia deserunt mollit anim id est laborum:

```bash
docker --context rocinante build -f web/Dockerfile \
    --build-arg SITE_BUILD=$(git rev-parse --short HEAD) \
    -t lightcone-web:latest web
```

And a fence with no language at all, which should render as plain text rather than as whatever
the highlighter guesses:

```
  1a lc-spacetime ─┐
                   ├─> 2 photometry ─> 3 PREMISE
  1b em-spectra ───┘
```
