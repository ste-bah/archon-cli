---
name: cancel-ralph
description: Cancel the active Ralph loop by removing its state file; use when the user wants to stop a running /ralph-loop.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/ralph-loop (Apache-2.0)
---
> Ported from the Claude Code `ralph-loop` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/ralph-loop), Apache-2.0).

# Cancel Ralph

To cancel the Ralph loop (use only Bash for the file checks/removal and Read for the state file):

1. Check if `.archon/ralph-loop.local.md` exists using Bash: `test -f .archon/ralph-loop.local.md && echo "EXISTS" || echo "NOT_FOUND"`

2. **If NOT_FOUND**: Say "No active Ralph loop found."

3. **If EXISTS**:
   - Read `.archon/ralph-loop.local.md` to get the current iteration number from the `iteration:` field
   - Remove the file using Bash: `rm .archon/ralph-loop.local.md`
   - Report: "Cancelled Ralph loop (was at iteration N)" where N is the iteration value
