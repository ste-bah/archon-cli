---
name: land-branch
description: Use when a branch or worktree's work is finished and needs to go somewhere — merging, opening a PR, or throwing it away. Also when several agent worktrees are outstanding and you do not know which are worth keeping.
---

# Landing a branch

Work that is finished but unmerged is not finished. It is a liability that gets
harder to land every day the base moves.

## First: is it actually done?

Run `/verify-done` before anything here. Landing unverified work is how a
broken main happens, and "the tests passed earlier" is not a verification.

If the board has `gaps_remain` items for this run, the turn will not end anyway
— close or decline them first.

## Review what is outstanding

```
/worktrees list
```

Shows every agent worktree, its branch, whether its owner is still alive, and
its diff **against the merge base** — not the base tip. That distinction
matters: measuring against the tip attributes everyone else's commits to this
branch and makes a two-file change look like a hundred-file one.

`/worktrees sizes` if you are short on disk. Build directories are usually the
bulk of it.

## Choose

Four honest options. Pick deliberately — drifting into "keep" by not deciding
is how twenty stale worktrees accumulate.

**Merge** — `/worktrees merge <name>`. Work is verified and wanted. Always
explicit; nothing merges itself.

**PR** — `/pr`. Same, but someone else should look first. Default for anything
touching a shared contract, a migration, or security.

**Keep** — `/worktrees keep <name>`. Not ready, still wanted. Say in the commit
or on the board what it is waiting for, or you will not know in a week.

**Discard** — `/worktrees discard <name>`. The experiment answered its
question, or the approach was wrong. Discarding a failed approach is a result,
not a waste — record what it ruled out before you drop it.

## Then clean up

```
/worktrees prune
```

Filters on liveness, not age: a worktree whose owner is gone is prunable
whether it was made ten minutes or ten days ago. A long-running agent's
worktree is safe.

Delete the branch once it is merged, locally and on the remote. Merged branches
that linger make `git branch` useless for telling you what is actually in
flight.

## What gets learned

`/worktrees merge` and `discard` write a trace row per isolated agent, pairing
what was true at spawn — declared writes, claim overlaps, isolation tier —
against what actually happened.

Merge outcomes are ground truth. A merge either conflicted or it did not; no
labeller is involved and no judgement is being trusted. That makes conflict
prediction the one signal here that trains honestly, which is why merging
through this path is worth more than merging by hand.

## Next

Landed? If more plan remains, back to `/execute-plan`. If the work is done,
close the issue and delete the branch.
