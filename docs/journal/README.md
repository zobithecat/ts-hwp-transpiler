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

## Reverted-decision template

When something gets built, validated, then thrown out, file a separate
entry named `YYYY-MM-DD-<topic>-reverted.md`. Specifically:

```markdown
# YYYY-MM-DD — <thing> tried, reverted

**Context**: what motivated the attempt; which Open question it closes.

**Built**: one paragraph on what shipped (and the commit that did,
since reset).

**Why reverted**: the axis the attempt failed on. Round-trip safety,
LLM readability, scope creep — be specific about *which* goal it
violated, not just "felt wrong".

**Decision**: the new resting state.

**Consequence**: who else is affected; whether the reverted approach
should ever be reconsidered (and under what condition).

**Open**: what's left to figure out.
```

Reverts are valuable evidence — they prevent the same idea from being
re-tried in a later session without the prior context. `2026-04-22-
rowspan-marker-reverted.md` is the format reference.

## Live reference docs

For knowledge that accumulates across sessions (binary spec details,
hwplib porting map, current session pointer), use `docs/memory/`
instead of journal entries. Journal = "what we decided, when, and why";
memory = "what is true now". Update memory when state changes; never
amend it for opinion.
