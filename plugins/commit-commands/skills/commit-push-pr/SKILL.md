---
name: commit-push-pr
description: Commit the current changes, push the branch, and open a GitHub pull request with gh in one step; use when work is ready for a PR.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/commit-commands (Apache-2.0)
---
> Ported from the Claude Code `commit-commands` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/commit-commands), Apache-2.0).

Note: any user arguments arrive appended to the end of this prompt; treat them as extra instructions (e.g. a branch name, PR title, or scope hints).

Tool constraint: use only `git checkout`, `git add`, `git status`, `git commit`, `git push`, `gh pr create`, and the read-only context commands below via Bash — no other tools.

## Context

Gather the following context first by running these commands (in a single message):

- Current git status: `git status`
- Current git diff (staged and unstaged changes): `git diff HEAD`
- Current branch: `git branch --show-current`

## Your task

Based on the above changes:

1. Create a new branch if on main
2. Create a single commit with an appropriate message
3. Push the branch to origin
4. Create a pull request using `gh pr create`
5. You have the capability to call multiple tools in a single response. You MUST do all of the above in a single message. Do not use any other tools or do anything else. Do not send any other text or messages besides these tool calls.
