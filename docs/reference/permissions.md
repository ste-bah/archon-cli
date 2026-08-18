# Permissions

archon-cli enforces a tool-level permission model on every tool call that touches the filesystem, shell, or network.

## The 7 canonical modes

Source of truth: `crates/archon-permissions/src/mode.rs:8` enum + `:74-83` FromStr.

| Mode | Behaviour |
|---|---|
| `default` | Prompt user for risky/dangerous operations (legacy alias: `ask`) |
| `acceptEdits` | Auto-allow file edits (Read/Write/Edit/Glob/Grep), prompt for Bash |
| `plan` | Blocks working-tree mutations by default; retained process-state controls are explicitly allowlisted |
| `auto` | Heuristic-based: auto-approve safe, prompt risky, warn dangerous |
| `dontAsk` | Auto-allow everything except `always_deny` rules |
| `bubble` | Auto-approve within sandbox limits (stricter than `dontAsk`, looser than `bypassPermissions`) |
| `bypassPermissions` | Skip all permission checks entirely (legacy alias: `yolo`) |

## Setting the mode

### CLI

```bash
archon --permission-mode default
archon --permission-mode auto
archon --permission-mode bypassPermissions
```

### Config file

```toml
[permissions]
mode = "default"          # default | acceptEdits | plan | auto | dontAsk | bubble | bypassPermissions
```

### In session (TUI)

```
/permissions auto
/sandbox                  # Toggle Bubble-mode sandbox
```

### Plan Mode approval and restoration

Plan Mode blocks working-tree mutations by default while retaining explicit process-state controls: `TaskCreate`, `TaskUpdate`, and `Agent`. Agent model/tool actions remain subject to Plan Mode and preflight boundaries.

A structured `ExitPlanMode` submission requires approval before Plan Mode is exited. Interactive approval offers three outcomes: `approve`, `approve-edits`, or `edit`; `reject: <reason>` rejects the structured exit and retains Plan Mode, so Write and other working-tree mutations remain blocked.

`/plan off`, `/plan exit`, and `/plan done` are explicit user commands that exit Plan Mode directly and restore `default` without structured plan approval.

A normal approval restores the recorded pre-plan mode through the safe restoration path. If no mode was recorded, the fallback is `default`, not `auto`; an unapproved `bypassPermissions` mode is also downgraded to `default`. `approve-edits` restores `acceptEdits` explicitly. Plan-linked task rows and their completion evidence persist, while unrelated manual tasks are process-scoped.

When no interactive AskUser channel is available, noninteractive Plan Mode approval defaults to approve. Set `[context].noninteractive_plan_approval = "reject"` to fail closed; any other value uses the approving default.


Beyond the mode, fine-grained rules in `[permissions]`:

```toml
[permissions]
mode = "default"
always_allow = ["Read:*", "Glob:*", "Grep:*"]
always_deny = ["Bash:rm -rf*", "Write:/etc/*"]
always_ask = ["Bash:git push*"]
allow_paths = ["/home/user/project"]
deny_paths = ["/etc", "/.ssh"]
sandbox = false                                   # When true: read-only enforcement
```

Rule format:
- `Tool:pattern` — pattern matches the tool's primary argument (path for file tools, command for Bash, URL for WebFetch)
- Patterns support glob (`*`, `**`), exact match, and prefix
- Order: `always_deny` > `always_ask` > `always_allow` > mode default

## CLI overrides

| Flag | Purpose |
|---|---|
| `--permission-mode <MODE>` | Runtime mode override |
| `--dangerously-skip-permissions` | Equivalent to `bypassPermissions` |
| `--allow-dangerously-skip-permissions` | Allow `bypassPermissions` in mode-cycle hotkey |
| `--sandbox` | Enforce `Bubble` sandbox (auto-approve reads, restrict writes) |

## Per-tool classifier

`Bash`, `PowerShell`, and `RemoteTrigger` use `crates/archon-permissions/src/classifier.rs` to classify each command at dispatch time:

- Read-only commands (`ls`, `cat`, `grep`, `git status`) → Safe
- Mutating commands (`rm`, `mv`, `>`, redirections) → Risky
- Network commands (`curl`, `wget`, `ssh`, `scp`) → Risky
- Destructive patterns (`rm -rf /`, `git push --force`, `dd of=`) → Always denied

The classifier respects rule lists — an `always_allow = ["Bash:rm /tmp/*"]` rule overrides the default Risky classification for matching commands.

## How agents inherit modes

Subagents inherit the parent's permission mode by default. Override per-spawn:

```rust
Agent {
    name: "code-reviewer",
    permission_mode: Some(PermissionMode::Plan),  // Block working-tree mutation; retain Plan-safe controls
    ...
}
```

Or via the slash command:
```
/run-agent code-reviewer --mode plan "review crates/archon-llm"
```

The `Agent` tool itself is auto-approved in `default` mode (PRD-AGENTS-001 Option B); dangerous downstream tools (Bash/Write/Edit) the subagent invokes still inherit gating from the subagent's effective mode.

## Sandboxing

When `[permissions] sandbox = true` (or `--sandbox` flag), additional read-only enforcement applies:
- Write/Edit/ApplyPatch are denied entirely
- Bash is restricted to the classifier's Safe set
- Any tool that can mutate state is gated

Combine with `Bubble` mode for a strict per-agent sandbox.

## Inspecting current state

In the TUI:
```
/permissions              # Show current mode + rules
/denials                  # Show denied permissions in current session
```

CLI:
```bash
archon --permission-mode default --print "list files in /etc"
# Returns: "Permission denied: Bash:ls /etc — would require auto/dontAsk/bypass mode"
```

## Examples

### Strict review workflow

```toml
[permissions]
mode = "plan"
allow_paths = ["${WORKDIR}"]
sandbox = true
always_deny = ["WebFetch:*", "RemoteTrigger:*"]
```

Working-tree mutations and network access are blocked; Plan-safe process-state controls remain available. Useful for security reviews of unfamiliar code.

### Trusted automation

```toml
[permissions]
mode = "auto"
always_deny = ["Bash:rm -rf*", "Bash:dd*", "Bash:*format*"]
allow_paths = ["${WORKDIR}", "${HOME}/.cache/archon"]
```

Heuristic auto-approval with hard guards against destructive commands.

### CI / unattended pipelines

```toml
[permissions]
mode = "dontAsk"
always_deny = ["Bash:rm -rf /", "Write:/etc/*"]
```

Or via flag: `archon --permission-mode dontAsk -p "task description"`.

## See also

- [Tools](tools.md) — per-tool default permission level
- [Configuration](config.md) — full `[permissions]` schema
- [CLI flags](cli-flags.md) — every command-line flag
