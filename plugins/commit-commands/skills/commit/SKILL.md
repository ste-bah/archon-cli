---
name: commit
description: Create a single git commit from the current changes with an appropriate message; use when you want staged/unstaged work committed.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/commit-commands (Apache-2.0)
---
> Ported from the Claude Code `commit-commands` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/commit-commands), Apache-2.0).

Note: any user arguments arrive appended to the end of this prompt; treat them as extra instructions for the commit (e.g. scope hints or a message to use).

Tool constraint: use only `git add`, `git status`, `git commit`, and the read-only context commands below via Bash — no other tools.

## Context

Gather the following context first by running these commands (in a single message):

- Current git status: `git status`
- Current git diff (staged and unstaged changes): `git diff HEAD`
- Current branch: `git branch --show-current`
- Recent commits: `git log --oneline -10`

## Your task

Based on the above changes, create a single git commit.

You have the capability to call multiple tools in a single response. Stage and create the commit using a single message. Do not use any other tools or do anything else. Do not send any other text or messages besides these tool calls.
