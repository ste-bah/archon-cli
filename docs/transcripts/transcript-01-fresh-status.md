# Transcript 01 — Fresh DB Status

## Command

```
$ archon behaviour status
```

## Output

```
warning: unrecognized environment variable: ANTHROPIC_SMALL_FAST_MODEL
warning: unrecognized environment variable: ANTHROPIC_BASE_URL
warning: unrecognized environment variable: ANTHROPIC_MODEL
Learning System Status
======================
Learning events: 0 total (0 false completions)
Proposals:  0 total (0 pending, 0 applied, 0 denied, 0 rolled back)
```

## Verification

- `archon behaviour status` returns learning event count (0), false completion count (0), and proposal breakdown (all zeros)
- No errors, no crashes
- CLI opens learning.db automatically, creates it if missing, ensures schema
- Env-var warnings are from config-level logging and are non-fatal

## Date Captured

2026-05-03
