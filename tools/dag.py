#!/usr/bin/env python3
"""Read a task graph written as markdown blocks, and say what it allows.

Usage: tools/dag.py <plan.md> check | ready | waves | graph [--write]

A task is a heading `### <ID> · <title>` followed by list lines, of which three are read:

    - status: todo | active <branch> | done <PR or commit> | dropped <why>
    - needs: <ID>, <ID> ...          (or `—` for none)
    - touches: `path`, `path` ...    (files or directories the task expects to change)

`check` validates ids, dependencies and cycles. `ready` lists todo tasks whose needs are all done,
and warns where one would touch a path an active task is touching. `waves` prints the longest-path
layering, which is the most concurrency the graph allows. `graph` prints a mermaid diagram of the
structure; `--write` replaces the block between the plan's `<!-- graph -->` markers with it.
"""
import re
import sys
from pathlib import Path

HEAD = re.compile(r"^### ([A-Z]+[0-9]+) · (.+)$")
FIELD = re.compile(r"^- (status|needs|touches): *(.*)$")
START, END = "<!-- graph -->", "<!-- /graph -->"


def parse(text):
    tasks, order, current = {}, [], None
    for number, line in enumerate(text.splitlines(), 1):
        head = HEAD.match(line)
        if head:
            ident = head.group(1)
            if ident in tasks:
                sys.exit(f"line {number}: {ident} is defined twice")
            current = tasks[ident] = {"title": head.group(2), "status": None, "needs": [], "touches": []}
            order.append(ident)
            continue
        if line.startswith("#"):
            current = None
            continue
        field = FIELD.match(line)
        if current is None or not field:
            continue
        key, value = field.groups()
        if key == "status":
            current["status"] = value.split()[0] if value else None
        elif key == "needs":
            current["needs"] = [n.strip() for n in value.split(",") if n.strip() not in ("", "—", "-")]
        else:
            current["touches"] = [t.strip().strip("`") for t in value.split(",") if t.strip()]
    return tasks, order


def check(tasks):
    problems = []
    for ident, task in tasks.items():
        if task["status"] not in ("todo", "active", "done", "dropped"):
            problems.append(f"{ident}: status is {task['status']!r}")
        for need in task["needs"]:
            if need not in tasks:
                problems.append(f"{ident}: needs {need}, which does not exist")
    state = {}

    def visit(ident, path):
        if state.get(ident) == "open":
            problems.append("cycle: " + " -> ".join(path + [ident]))
            return
        if state.get(ident) == "closed":
            return
        state[ident] = "open"
        for need in tasks[ident]["needs"]:
            if need in tasks:
                visit(need, path + [ident])
        state[ident] = "closed"

    for ident in tasks:
        visit(ident, [])
    return problems


def waves(tasks, order):
    depth = {}

    def level(ident):
        if ident not in depth:
            depth[ident] = 1 + max((level(n) for n in tasks[ident]["needs"]), default=-1)
        return depth[ident]

    for ident in order:
        level(ident)
    layers = {}
    for ident in order:
        layers.setdefault(depth[ident], []).append(ident)
    return [layers[k] for k in sorted(layers)]


def overlaps(a, b):
    return any(x == y or x.startswith(y.rstrip("/") + "/") or y.startswith(x.rstrip("/") + "/") for x in a for y in b)


def mermaid(tasks, order):
    lines = ["```mermaid", "graph LR"]
    for ident in order:
        lines.append(f'  {ident}["{ident} {tasks[ident]["title"]}"]')
    for ident in order:
        for need in tasks[ident]["needs"]:
            lines.append(f"  {need} --> {ident}")
    lines.append("```")
    return "\n".join(lines)


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    path, command = Path(sys.argv[1]), sys.argv[2]
    text = path.read_text()
    tasks, order = parse(text)
    problems = check(tasks)
    if command == "check":
        print("\n".join(problems) or f"{len(tasks)} tasks, no problems")
        sys.exit(1 if problems else 0)
    if problems:
        sys.exit("\n".join(problems))
    done = {i for i, t in tasks.items() if t["status"] in ("done", "dropped")}
    if command == "ready":
        active = [i for i in order if tasks[i]["status"] == "active"]
        for ident in order:
            task = tasks[ident]
            if task["status"] != "todo" or not set(task["needs"]) <= done:
                continue
            clash = [a for a in active if overlaps(task["touches"], tasks[a]["touches"])]
            note = f"   (touches what {', '.join(clash)} is touching)" if clash else ""
            print(f"{ident}  {task['title']}{note}")
    elif command == "waves":
        for number, layer in enumerate(waves(tasks, order)):
            print(f"wave {number}: {', '.join(layer)}")
    elif command == "graph":
        diagram = mermaid(tasks, order)
        if "--write" in sys.argv:
            if START not in text or END not in text:
                sys.exit(f"no {START} ... {END} block in {path}")
            head, rest = text.split(START, 1)
            _, tail = rest.split(END, 1)
            path.write_text(f"{head}{START}\n{diagram}\n{END}{tail}")
        else:
            print(diagram)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
