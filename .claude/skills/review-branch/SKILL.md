---
name: review-branch
description: Review a branch's whole change against master and its design docs, including anything merged into it, and print the review as a single block to paste to the agent doing the work. Runs in its own subagent. Use when asked to review a branch, or invoked as /review-branch [branch] [base].
context: fork
agent: general-purpose
---

# Review a branch

Arguments: `$ARGUMENTS`. The first is the branch to review, the second the base to review it
against. With no branch, review the checked-out one (`HEAD`); with no base, `origin/master`.

A review is judgement about code. **Read the code; do not run test suites or builds.** They cost
the user's budget and the user can run them. `git`, `grep` and reading files are fine.

## 1. Find the change

```bash
git fetch -q origin
git log --oneline <base>..<branch>          # every commit, merges included
git diff --stat <base>...<branch>
git diff <base>...<branch>
```

The three-dot diff is the change: everything the branch adds over the base, including branches
merged into it. For a merge commit, review both halves and the merge itself: `git show
--first-parent <merge>` is what the merge changed on this branch, and code that adapts one half
to the other is where the two can disagree. If the branch does not exist or the base does not
contain the merge base you expect, ask the user rather than guessing.

## 2. Learn what it is for

Read every commit message; they state the intent, and a message's claims are part of what is
reviewed. Then read the design. The docs are the authority: read in full every doc section the
diff edits, and follow links from the diff to any doc a new comment or name points at. Where the
code and a doc disagree, either the code is wrong or the branch must update the doc, **in the
same branch**. Read `CLAUDE.md` and `AGENTS.md` once for the conventions.

## 3. Review

Check, in this order:

- **Intent.** Each commit does what its message and the docs say. Every claim a test makes is
  shown by a test that would fail if the mechanism broke. Anything the change defers or leaves
  open is recorded in a doc, not only in a commit message; a deferral nobody records is a
  **major**.
- **Against the docs.** Every value, name, unit, formula and table row the docs give, checked
  against the code. Do the arithmetic for derived numbers yourself. If the branch edits a doc,
  check that the edit is right rather than just convenient.
- **Correctness.** Bugs, wrong units or frames, numerical hazards, placeholder values that fail
  quietly when read early, state that is not saved and what a restart does to it, and where two
  merged halves meet.
- **Repo conventions.** `CLAUDE.md` and `AGENTS.md`: comment rules, the 1000-line module cap
  (tests excluded), the crate dependency invariants, and the format numbers that must move when a
  stored or wire shape changes. **British spelling is a major**, never a repo convention to
  defend. Lightcone has no players, so asking for compatibility shims is itself a defect.

Rank findings **Major** (must fix before merge), **Minor** (fix now or record as a follow-up),
**Nit**. Each finding gives the place (`path:line`), what the doc or intent says versus what the
code does, and a concrete fix. Drop anything you could not confirm by reading.

## 4. Output

Your final message is the review. Make it **one fenced `text` block**, so it pastes cleanly into
another agent's session. Write it to that agent in the second person, with plain `path:line`
references (no markdown links, which don't survive the paste). Shape:

```text
Review of <branch> (<n> commits<, including the merge of X>) against <base> and <docs read>.

Verdict: <one or two sentences: mergeable or not, and what it hangs on>.

MAJOR
1. <title>. <path:line>. <what the doc or intent says vs. what the code does>. Fix: <what to do>.

MINOR
2. ...

NIT
3. ...

Confirmed correct: <what you checked and found right, briefly, so it isn't re-litigated>.
```

After the block, add one line for the user saying whether you would merge it.
