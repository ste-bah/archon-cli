# Hooks

Hooks are shell commands that execute in response to lifecycle events. Defined in `config.toml`, `.archon/settings.json`, or `.claude/settings.json` (backward compat).

## Hook events

`Setup`, `SessionStart`, `SessionEnd`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PreCompact`, `PostCompact`, `ConfigChange`, `CwdChanged`, `FileChanged`, `InstructionsLoaded`, `UserPromptSubmit`, `Stop`, `SubagentStart`, `SubagentStop`, `TaskCreated`, `TaskCompleted`, `PermissionDenied`, `PermissionRequest`, `Notification`.

## Definition formats

### TOML (config.toml)

```toml
[[hooks.pre_tool_use]]
command = "scripts/check-dangerous-patterns.sh"
timeout = 30
blocking = true                     # exit code 2 cancels the tool call

[[hooks.session_start]]
command = "git status --short"
timeout = 5

[[hooks.session_end]]
command = "scripts/log-session.sh"
timeout = 10
```

### JSON (`.archon/settings.json`) with structured matchers

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": { "tool_name": "Bash" },
        "hooks": [{ "type": "command", "command": "scripts/audit-bash.sh" }]
      },
      {
        "matcher": { "tool_name": "Write", "path_regex": "^/etc/" },
        "hooks": [{ "type": "command", "command": "scripts/block.sh", "blocking": true }]
      }
    ]
  }
}
```

Matchers can filter by tool name, path regex, command regex, agent name, or any other event field.

## Event data

Hooks receive event data via JSON on stdin:

```json
{
  "event": "PreToolUse",
  "session_id": "abc-123",
  "tool_name": "Bash",
  "tool_args": { "command": "rm -rf /tmp/foo" },
  "timestamp_ms": 1714000000000,
  "agent_name": "main",
  "permission_mode": "default"
}
```

Hook scripts can:
- Inspect the event JSON
- Run their own logic (audit, log, validate)
- Short-circuit the operation by exiting with code 2 (only effective when `blocking = true`)

## Built-in hook patterns

### Pre-commit auditor

```bash
#!/bin/bash
# scripts/audit-bash.sh
read EVENT
COMMAND=$(echo "$EVENT" | jq -r '.tool_args.command')
if echo "$COMMAND" | grep -qE 'rm -rf /|dd if=/dev'; then
    echo "Blocked dangerous command" >&2
    exit 2
fi
```

### Session start logger

```bash
#!/bin/bash
# scripts/log-session.sh
read EVENT
SESSION_ID=$(echo "$EVENT" | jq -r '.session_id')
echo "[$(date)] Session $SESSION_ID started" >> ~/.local/share/archon/session.log
```

### Auto-format on file write

```bash
#!/bin/bash
# scripts/format-on-write.sh
read EVENT
FILE=$(echo "$EVENT" | jq -r '.tool_args.path')
case "$FILE" in
    *.rs)  rustfmt "$FILE" ;;
    *.py)  ruff format "$FILE" ;;
    *.ts)  prettier --write "$FILE" ;;
esac
```

## Slash commands

```
/hooks                            # list registered hooks
/hooks <event>                    # filter by event
/hooks add <event> <command>      # register a new hook (writes to .archon/settings.json)
```

## Disabling hooks

```bash
archon --bare                     # skip all hooks for this invocation
```

Or per-event in config:
```toml
[hooks.pre_tool_use]
disabled = true
```

## See also

- [Plugins](plugins.md) — heavier-weight extensions (registered tools/skills)
- [Configuration](../reference/config.md) — config file precedence
