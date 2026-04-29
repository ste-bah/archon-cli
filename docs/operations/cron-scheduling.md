# Cron & scheduling

Schedule recurring or one-shot background tasks via standard 5-field cron expressions. archon-cli's scheduler runs in-process when archon is active and persists schedules to disk for cross-session survival.

## Tools

| Tool | Purpose |
|---|---|
| `CronCreate` | Schedule a task with a cron expression + description |
| `CronList` | List all scheduled tasks |
| `CronDelete` | Remove a scheduled task by ID |

## Slash command

```
/schedule "every morning at 9am, run the test suite and summarize failures"
```

The `/schedule` skill delegates to `CronCreate`, which parses natural language into cron syntax (e.g., `0 9 * * *`) and stores the task.

## Cron expression format

Standard 5-field POSIX cron:

```
* * * * *
| | | | |
| | | | +--- Day of week (0-6, Sun=0)
| | | +----- Month (1-12)
| | +------- Day of month (1-31)
| +--------- Hour (0-23)
+----------- Minute (0-59)
```

Common patterns:

| Expression | Meaning |
|---|---|
| `0 9 * * *` | Every day at 9:00 AM |
| `*/15 * * * *` | Every 15 minutes |
| `0 */2 * * *` | Every 2 hours |
| `0 0 * * 0` | Every Sunday at midnight |
| `0 9 * * 1-5` | Weekdays at 9:00 AM |
| `30 17 * * 5` | Friday at 5:30 PM |

## Tool examples

### Create

```jsonc
// Tool: CronCreate
{
  "expression": "0 9 * * *",
  "description": "Daily test summary",
  "task": "Run cargo test --workspace, summarize any failures"
}
```

### List

```jsonc
// Tool: CronList
// Returns array of { id, expression, description, last_run, next_run }
```

### Delete

```jsonc
// Tool: CronDelete
{
  "id": "cron-abc123"
}
```

## Storage

Schedules persist to `~/.local/share/archon/cron.db` (CozoDB). The scheduler resumes pending schedules on archon startup.

## Limitations

- Schedules only fire while archon-cli is running (not a system-level daemon)
- For 24/7 scheduling, run `archon serve` in a long-running process or use system cron + `archon -p`
- Maximum task runtime per fire: 5 minutes (configurable via `[orchestrator] timeout_secs`)

## System cron alternative

For schedules that must run when archon isn't open, use system cron + print mode:

```cron
# Daily 9 AM
0 9 * * * /usr/local/bin/archon -p "summarize yesterday's commits" --no-session-persistence
```

Output goes to wherever your cron output is configured (typically email or `>>` to a file).

## See also

- [Tools](../reference/tools.md) — `CronCreate`, `CronList`, `CronDelete`
- [Skills](../reference/skills.md) — `/schedule` skill
- [Configuration](../reference/config.md) — `[orchestrator]` section
