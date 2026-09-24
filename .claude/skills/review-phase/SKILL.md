---
name: review-phase
description: Review one task from a lightcone plan (a phase ID like K2, F4 or H1) against its design docs, and print the review as a single block to paste to the agent doing the work. Use when asked to review a phase or task by ID, or invoked as /review-phase <id>.
---

# Review a phase

The argument is a task ID from a plan in `lightcone/docs/plans/`, like `k2` or `F4`. Case does
not matter. If there is no argument, ask for one.

A review is judgement about code. **Read the code; do not run test suites or builds.** They cost
the user's budget and the user can run them. `git`, `grep` and reading files are fine.

## 1. Find the task

```bash
grep -n "^### K2 ·" lightcone/docs/plans/*.md
```

Read the whole block: `status`, `needs`, `touches`, `read`, `deliver`, `done when`. Read the plan's
**How to use this file** section once too. It holds the rules every task follows.

## 2. Find the branch

The claim commit sets the status line to `- status: active <branch>`, and finishing it sets it to
`- status: done <PR or commit>`. Fetch first, then try these in order:

```bash
git fetch -q origin
git log --all --oneline --grep="Claim K2"                       # the claim commit
git branch -a --contains <claim commit> | grep -v master        # the branch it sits on
```

If the status on `master` says `done #N`, `gh pr view N --json headRefName,state` names the
branch. If two branches hold the claim, or none does, ask the user which to review. Don't guess.

The change is `git diff master...<branch>`. Also read `git log master..<branch>`.

## 3. Read the design

Read every section the task's `read` line names, in full. Then follow the links from the diff:
any doc the change edits, and any doc a new comment or name points at. The docs are the
authority. Where the code and a doc disagree, either the code is wrong or the PR must update the
doc. The plan requires the doc to be updated **in the same PR**.

## 4. Review

Check, in this order:

- **The task's contract.** Every item in `deliver` is there. `done when` is actually shown, by a
  test that would fail if the mechanism broke. Nothing lands outside `touches` without a reason.
  The status line is `done <PR>`, and the PR edits only its own task's block of the plan.
- **Against the docs.** Every value, name, unit, formula and table row the docs give, checked
  against the code. Do the arithmetic for derived numbers yourself. If the PR edits a doc, check
  that the edit is right rather than just convenient.
- **Against the graph.** Anything the change defers ("until F9", "anchored by H2") must be owned
  by a task whose `deliver` line actually says so, and that task must be able to do it: its
  `needs` must already provide what the deferred work depends on. A deferral nobody owns is a
  **major**.
- **Correctness.** Bugs, wrong units or frames, numerical hazards, placeholder values that fail
  quietly when read early.
- **Repo conventions.** `CLAUDE.md` and `AGENTS.md`: comment rules, the 1000-line module cap
  (tests excluded), the crate dependency invariants. **British spelling is a major**, never a
  repo convention to defend. Lightcone has no players, so asking for protocol-version bumps or
  compatibility shims is itself a defect.

Rank findings **Major** (must fix before merge), **Minor** (fix now or in the named later task),
**Nit**. Each finding gives the place (`path:line`), what the doc says versus what the code
does, and a concrete fix. Drop anything you could not confirm by reading.

## 5. Output

Print **one fenced `text` block**, so it pastes cleanly into another agent's session. Write it
to that agent in the second person, with plain `path:line` references (no markdown links, which
don't survive the paste). Shape:

```text
Review of <ID> · <task title> (<branch>, <n> commits) against <docs read>.

Verdict: <one or two sentences: mergeable or not, and what it hangs on>.

MAJOR
1. <title>. <path:line>. <what the doc says vs. what the code does>. Fix: <what to do>.

MINOR
2. ...

NIT
3. ...

Confirmed correct: <what you checked and found right, briefly, so it isn't re-litigated>.
```

After the block, add one line for the user saying whether you would merge it.
