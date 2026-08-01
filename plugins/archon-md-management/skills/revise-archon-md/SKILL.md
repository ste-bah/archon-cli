---
name: revise-archon-md
description: Update ARCHON.md with learnings from the current session. Use at the end of a session, or whenever the session revealed missing project context worth capturing.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/claude-md-management (Apache-2.0)
---
> Ported from the Claude Code `claude-md-management` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/claude-md-management), Apache-2.0).

Review this session for learnings about working with Archon in this codebase. Update ARCHON.md with context that would help future Archon sessions be more effective.

Tool constraint: this task should only need file discovery (Glob, or the `find` below), Read, and Edit — do not make unrelated changes.

## Step 1: Reflect

What context was missing that would have helped Archon work more effectively?
- Bash commands that were used or discovered
- Code style patterns followed
- Testing approaches that worked
- Environment/configuration quirks
- Warnings or gotchas encountered

## Step 2: Find ARCHON.md Files

```bash
find . -name "ARCHON.md" -o -name ".archon.local.md" 2>/dev/null | head -20
```

Decide where each addition belongs:
- `ARCHON.md` - Team-shared (checked into git)
- `.archon.local.md` - Personal/local only (gitignored)

## Step 3: Draft Additions

**Keep it concise** - one line per concept. ARCHON.md is part of the prompt, so brevity matters.

Format: `<command or pattern>` - `<brief description>`

Avoid:
- Verbose explanations
- Obvious information
- One-off fixes unlikely to recur

## Step 4: Show Proposed Changes

For each addition:

```
### Update: ./ARCHON.md

**Why:** [one-line reason]

\`\`\`diff
+ [the addition - keep it brief]
\`\`\`
```

## Step 5: Apply with Approval

Ask if the user wants to apply the changes. Only edit files they approve.
