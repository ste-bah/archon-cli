---
name: ralph-loop
description: Start a Ralph Wiggum self-referential development loop in the current session; use when the user wants the agent to iterate autonomously on a task until completion, e.g. "keep iterating until the tests pass".
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/ralph-loop (Apache-2.0)
---
> Ported from the Claude Code `ralph-loop` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/ralph-loop), Apache-2.0).

# Ralph Loop Command

Usage hint: `/ralph-loop PROMPT [--max-iterations N] [--completion-promise TEXT]` - the arguments are appended to the end of this prompt.

Execute the setup script to initialize the Ralph loop. Run it with the Bash tool, passing along the arguments appended to the end of this prompt (if any) exactly as given:

```bash
bash .archon/plugins/ralph-loop/scripts/setup-ralph-loop.sh <arguments appended to the end of this prompt>
```

(For a user-global install, the script is at `~/.archon/plugins/ralph-loop/scripts/setup-ralph-loop.sh` instead. Do not run any other commands to set up the loop; the script creates the `.archon/ralph-loop.local.md` state file itself. If the script reports an error, relay it to the user and stop.)

Then please work on the task described in the arguments. When you try to exit, the Ralph loop will feed the SAME PROMPT back to you for the next iteration. You'll see your previous work in files and git history, allowing you to iterate and improve.

CRITICAL RULE: If a completion promise is set, you may ONLY output it when the statement is completely and unequivocally TRUE. Do not output false promises to escape the loop, even if you think you're stuck or should exit for other reasons. The loop is designed to continue until genuine completion.

Note for Archon: if the stop hook's continuation message tells you that no transcript is available for promise detection, the completion signal is instead setting `active: false` in the frontmatter of `.archon/ralph-loop.local.md` - and the same CRITICAL RULE applies: only set it when the promise statement is genuinely TRUE.
