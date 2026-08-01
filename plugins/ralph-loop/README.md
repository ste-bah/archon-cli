# Ralph Loop Plugin

> Ported from the Claude Code `ralph-loop` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/ralph-loop), Apache-2.0).

Implementation of the Ralph Wiggum technique for iterative, self-referential AI development loops in Archon.

## What is Ralph Loop?

Ralph Loop is a development methodology based on continuous AI agent loops. As Geoffrey Huntley describes it: **"Ralph is a Bash loop"** - a simple `while true` that repeatedly feeds an AI agent a prompt file, allowing it to iteratively improve its work until completion.

This technique is inspired by the Ralph Wiggum coding technique (named after the character from The Simpsons), embodying the philosophy of persistent iteration despite setbacks.

### Core Concept

This plugin implements Ralph using a **Stop hook** that intercepts the agent's exit attempts:

```bash
# You run ONCE:
/ralph-loop "Your task description" --completion-promise "DONE"

# Then Archon automatically:
# 1. Works on the task
# 2. Tries to exit
# 3. Stop hook blocks exit
# 4. Stop hook feeds the SAME prompt back
# 5. Repeat until completion
```

The loop happens **inside your current session** - you don't need external bash loops. The Stop hook in `scripts/stop-hook.sh` creates the self-referential feedback loop by blocking normal session exit.

This creates a **self-referential feedback loop** where:
- The prompt never changes between iterations
- The agent's previous work persists in files
- Each iteration sees modified files and git history
- The agent autonomously improves by reading its own past work in files

## Installation

Project-local install:

1. Copy each `skills/<skill>/` dir to `<project>/.archon/skills/<skill>/` (skills: `ralph-loop`, `cancel-ralph`, `ralph-help`).
2. Copy `scripts/` to `<project>/.archon/plugins/ralph-loop/scripts/`.
3. Enable the hooks (see below).

Or user-global: skills to `~/.config/archon/skills/` (or platform data dir + `archon/skills/`), scripts to `~/.archon/plugins/ralph-loop/scripts/`.

Or run `plugins/install.ps1 ralph-loop` / `plugins/install.sh ralph-loop` from the archon-cli repo root, then enable the hooks.

Restart archon after installing (skills load at startup).

## Enable hooks

Hooks are NOT auto-loaded from plugin directories. Merge `hooks/settings.snippet.json` into `<project>/.archon/settings.json` (create the file with that content if it doesn't exist; otherwise merge the `"hooks"` keys). The snippet registers the Stop hook with `"blocking": true` - required so that exit code 2 actually prevents the session from stopping.

If you installed user-globally, change the script path in the snippet from `.archon/plugins/ralph-loop/scripts/stop-hook.sh` to `~/.archon/plugins/ralph-loop/scripts/stop-hook.sh`.

The stop hook requires `bash`, `jq`, `perl`, `sed`, `awk`, and `grep` on PATH (all present in Git Bash on Windows).

## Quick Start

```bash
/ralph-loop "Build a REST API for todos. Requirements: CRUD operations, input validation, tests. Output <promise>COMPLETE</promise> when done." --completion-promise "COMPLETE" --max-iterations 50
```

The agent will:
- Implement the API iteratively
- Run tests and see failures
- Fix bugs based on test output
- Iterate until all requirements met
- Output the completion promise when done

## Skills

### /ralph-loop

Start a Ralph loop in your current session.

**Usage:**
```bash
/ralph-loop "<prompt>" --max-iterations <n> --completion-promise "<text>"
```

**Options:**
- `--max-iterations <n>` - Stop after N iterations (default: unlimited)
- `--completion-promise <text>` - Phrase that signals completion

### /cancel-ralph

Cancel the active Ralph loop.

**Usage:**
```bash
/cancel-ralph
```

### /ralph-help

Explain the plugin and its skills.

## Prompt Writing Best Practices

### 1. Clear Completion Criteria

❌ Bad: "Build a todo API and make it good."

✅ Good:
```markdown
Build a REST API for todos.

When complete:
- All CRUD endpoints working
- Input validation in place
- Tests passing (coverage > 80%)
- README with API docs
- Output: <promise>COMPLETE</promise>
```

### 2. Incremental Goals

❌ Bad: "Create a complete e-commerce platform."

✅ Good:
```markdown
Phase 1: User authentication (JWT, tests)
Phase 2: Product catalog (list/search, tests)
Phase 3: Shopping cart (add/remove, tests)

Output <promise>COMPLETE</promise> when all phases done.
```

### 3. Self-Correction

❌ Bad: "Write code for feature X."

✅ Good:
```markdown
Implement feature X following TDD:
1. Write failing tests
2. Implement feature
3. Run tests
4. If any fail, debug and fix
5. Refactor if needed
6. Repeat until all green
7. Output: <promise>COMPLETE</promise>
```

### 4. Escape Hatches

Always use `--max-iterations` as a safety net to prevent infinite loops on impossible tasks:

```bash
# Recommended: Always set a reasonable iteration limit
/ralph-loop "Try to implement feature X" --max-iterations 20

# In your prompt, include what to do if stuck:
# "After 15 iterations, if not complete:
#  - Document what's blocking progress
#  - List what was attempted
#  - Suggest alternative approaches"
```

**Note**: The `--completion-promise` uses exact string matching, so you cannot use it for multiple completion conditions (like "SUCCESS" vs "BLOCKED"). Always rely on `--max-iterations` as your primary safety mechanism.

## Philosophy

Ralph embodies several key principles:

### 1. Iteration > Perfection
Don't aim for perfect on first try. Let the loop refine the work.

### 2. Failures Are Data
"Deterministically bad" means failures are predictable and informative. Use them to tune prompts.

### 3. Operator Skill Matters
Success depends on writing good prompts, not just having a good model.

### 4. Persistence Wins
Keep trying until success. The loop handles retry logic automatically.

## When to Use Ralph

**Good for:**
- Well-defined tasks with clear success criteria
- Tasks requiring iteration and refinement (e.g., getting tests to pass)
- Greenfield projects where you can walk away
- Tasks with automatic verification (tests, linters)

**Not good for:**
- Tasks requiring human judgment or design decisions
- One-shot operations
- Tasks with unclear success criteria
- Production debugging (use targeted debugging instead)

## Real-World Results

- Successfully generated 6 repositories overnight in Y Combinator hackathon testing
- One $50k contract completed for $297 in API costs
- Created entire programming language ("cursed") over 3 months using this approach

## Windows Compatibility

The stop hook uses a bash script that requires Git for Windows to run properly.

**Issue**: On Windows, the `bash` command may resolve to WSL bash (often misconfigured) instead of Git Bash, causing the hook to fail with errors like:
- `wsl: Unknown key 'automount.crossDistro'`
- `execvpe(/bin/bash) failed: No such file or directory`

**Workaround**: Edit the merged hook entry in `.archon/settings.json` to use Git Bash explicitly:

```json
"command": "\"C:/Program Files/Git/bin/bash.exe\" .archon/plugins/ralph-loop/scripts/stop-hook.sh"
```

**Note**: Use `Git/bin/bash.exe` (the wrapper with proper PATH), not `Git/usr/bin/bash.exe` (raw MinGW bash without utilities in PATH).

## Differences from the Claude plugin

- **Blocking mechanism**: the Claude stop hook emitted `{"decision": "block", "reason": <prompt>, "systemMessage": ...}` JSON on stdout with exit 0. Archon does not interpret hook stdout JSON, so the port prints the system message and the continuation prompt to stderr and exits 2, with `"blocking": true` in the settings snippet. How prominently the fed-back prompt is surfaced to the agent depends on how the Archon version presents blocked-Stop hook output.
- **State file location**: `.claude/ralph-loop.local.md` → `.archon/ralph-loop.local.md` across the stop hook, setup script, and all skills.
- **Transcript-based promise detection is conditional**: Claude's Stop event always included `transcript_path`; Archon's documented Stop event does not. If `transcript_path` is present, the port's promise detection works exactly as upstream (JSONL parse, `<promise>` tag match). If absent, the hook skips promise detection, keeps looping, and its continuation message instructs the agent to signal genuine completion by setting `active: false` in the state file's frontmatter instead. The `active: false` exit path is new in this port (upstream wrote the field but never read it); the honesty rule for promises applies to it equally.
- **Session isolation is best-effort**: the setup script records `${ARCHON_SESSION_ID:-}` (the Claude original used `CLAUDE_CODE_SESSION_ID`). If Archon does not export a session-id env var, the field is empty and the isolation check is skipped - the pre-session_id legacy behavior of the original.
- **Skill mechanics**: the Claude command used inline `!`-execution and an `allowed-tools` frontmatter entry scoped to Bash-running the setup script under the Claude plugin root, plus `hide-from-slash-command-tool`. Archon skills are injected prompts, so `/ralph-loop` now instructs the agent to run the setup script with the Bash tool, and the tool restriction is stated as body prose. `hide-from-slash-command-tool` has no Archon equivalent and was dropped.
- **Command naming**: commands became skills `/ralph-loop`, `/cancel-ralph`, and `/ralph-help` (the source's `/help` was renamed to avoid colliding with the generic help command).
- **Hook registration**: Claude auto-loaded `hooks/hooks.json`; Archon requires merging `hooks/settings.snippet.json` into `.archon/settings.json` (see "Enable hooks").
- **Doc fixes carried over**: the source help command inconsistently wrote `.claude/.ralph-loop.local.md` (extra leading dot); the port consistently uses `.archon/ralph-loop.local.md`.

## Learn More

- Original technique: https://ghuntley.com/ralph/
- Ralph Orchestrator: https://github.com/mikeyobrien/ralph-orchestrator

## For Help

Run `/ralph-help` in Archon for detailed skill reference and examples.

## License

Apache License 2.0 (see LICENSE).
