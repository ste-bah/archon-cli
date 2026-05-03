# Transcript 02 — CLI Subcommand Surface Verification

## Command

```
$ archon behaviour --help
```

## Output

```
Manage governed learning behaviour

Usage: archon behaviour <COMMAND>

Commands:
  list-proposals      List behaviour proposals (alias: proposals)
  list-events         List learning events (optionally filtered by type)
  show                Show details for a proposal, event, or manifest version
  apply               Auto-apply a pending proposal (without human review)
  history             Show version history for a manifest kind
  generate-proposals  Generate proposals from recent learning events
  status              Show learning system status and statistics
  approve             Approve a pending proposal (human-in-the-loop)
  deny                Deny a pending proposal
  rollback            Rollback a manifest to a previous version
  help                Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

## Verification

- 10 subcommands listed (including `help`)
- 9 operational subcommands: list-proposals, list-events, show, apply, history, generate-proposals, status, approve, deny, rollback
- `list-proposals` shows deprecated alias `(alias: proposals)`
- Additional tests (run separately):
  - `archon behaviour proposals` → works via alias (same output as list-proposals)
  - `archon behaviour list-events --help` → shows `--event-type` flag
  - `archon behaviour list-events -e FalseCompletionDetected` → filters by type
  - `archon behaviour show nonexistent` → "No proposal, version, or event found with ID: nonexistent"
  - `archon behaviour history RetrievalProfile` → "No version history found for manifest kind: RetrievalProfile"

## Date Captured

2026-05-03
