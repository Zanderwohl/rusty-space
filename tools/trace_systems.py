#!/usr/bin/env python3
"""Where a frame's CPU went, per system, from a Bevy chrome trace.

Usage: tools/trace_systems.py <trace.json> [--skip N] [--top N] [--spans]

Build the client with `--features bevy/trace,bevy/trace_chrome` and run it with `--bench`;
`TRACE_CHROME=<path>` names the file. Each system's time is summed and divided by the number of
`Update` schedules seen, so the figures are milliseconds per frame. `--skip` drops that many
frames from the start, which is the warm-up `--frames` asked for.

Tracing costs about a microsecond a span, so these are for ranking, not for absolute numbers;
`--bench` without the features gives those. `--spans` adds every other span by name: the
cameras' render schedules, render-graph nodes and egui passes, which are not systems.
"""
import json
import re
import sys
from collections import defaultdict

SYSTEM = re.compile(r'^system: name="(.+)"$')
SCHEDULE = re.compile(r'^schedule: name=(.+)$')


def load(path):
    text = open(path).read().rstrip().rstrip(",")
    if not text.endswith("]"):
        text += "]"
    return json.loads(text)


def spans(events):
    """Complete spans as (name, tid, start_us, duration_us), from B/E pairs or X events."""
    stacks = defaultdict(list)
    for e in events:
        ph = e.get("ph")
        if ph == "X":
            yield e["name"], e.get("tid"), e["ts"], e.get("dur", 0.0)
        elif ph == "B":
            stacks[e.get("tid")].append(e)
        elif ph == "E":
            stack = stacks[e.get("tid")]
            if stack:
                begin = stack.pop()
                yield begin["name"], begin.get("tid"), begin["ts"], e["ts"] - begin["ts"]


def main():
    args = sys.argv[1:]
    path = args[0]
    skip = int(args[args.index("--skip") + 1]) if "--skip" in args else 0
    top = int(args[args.index("--top") + 1]) if "--top" in args else 40

    all_spans = sorted(spans(load(path)), key=lambda s: s[2])
    updates = [s for s in all_spans if SCHEDULE.match(s[0]) and SCHEDULE.match(s[0]).group(1) == "Update"]
    if len(updates) <= skip:
        sys.exit(f"only {len(updates)} Update schedules in the trace")
    start = updates[skip][2]
    frames = len(updates) - skip

    systems, schedules, other, count = defaultdict(float), defaultdict(float), defaultdict(float), defaultdict(int)
    for name, _, ts, dur in all_spans:
        if ts < start:
            continue
        if m := SYSTEM.match(name):
            systems[m.group(1)] += dur
        elif m := SCHEDULE.match(name):
            schedules[m.group(1)] += dur
        else:
            other[name[:120]] += dur
            count[name[:120]] += 1

    print(f"{frames} frames")
    print("\nschedules, ms per frame")
    for name, us in sorted(schedules.items(), key=lambda kv: -kv[1])[:16]:
        print(f"  {us / frames / 1e3:8.3f}  {name}")
    print("\nsystems, ms per frame")
    for name, us in sorted(systems.items(), key=lambda kv: -kv[1])[:top]:
        print(f"  {us / frames / 1e3:8.3f}  {name}")
    if "--spans" in args:
        print("\nother spans, ms per frame, times per frame")
        for name, us in sorted(other.items(), key=lambda kv: -kv[1])[:top]:
            print(f"  {us / frames / 1e3:8.3f}  x{count[name] / frames:5.1f}  {name}")


if __name__ == "__main__":
    main()
