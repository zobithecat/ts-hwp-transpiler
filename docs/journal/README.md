# Engineering Journal

Append-only log of architectural decisions, scope shifts, and learnings. Not
a PR changelog (git handles that), not a task list (see TaskList). What
belongs here:

- **Scope commitments and reversals** — why a direction was chosen, what
  alternatives were rejected, what the tradeoffs were.
- **Verified constraints** — "cfb does X, not Y", "hwplib rounds off
  padding bytes here", grounded in a specific test run or source read.
- **Dead ends with reasons** — if we tried something and backed out, record
  it so we don't repeat.
- **Open questions waiting on a human** — so the next session can pick them
  up without re-deriving.

What does NOT belong here:

- Commit messages, PR descriptions, or "what changed" summaries derivable
  from `git log`.
- Current task state — use TaskList.
- Step-by-step debugging traces — those die with the bug; only the
  takeaway-level conclusion is worth preserving.

## File layout

- One file per session / material decision, named `YYYY-MM-DD[-slug].md`.
- Entries are additive — amend a prior entry only to correct a factual
  error, not to reflect changed opinions (write a new entry instead).

## Entry template

```markdown
# YYYY-MM-DD — short title

**Context**: one paragraph, why we were working on this.

**Decision / Finding**: the durable bit.

**Why**: tradeoffs considered; alternatives rejected and why.

**Consequence**: what now depends on this — the "if this changes, these
things must be revisited" list.

**Open questions**: anything deferred.
```
