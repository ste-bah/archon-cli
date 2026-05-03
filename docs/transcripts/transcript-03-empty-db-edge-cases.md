# Transcript 03 — Empty-DB Edge Case Handling

## Empty list-events

```
$ archon behaviour list-events
No learning events found.
```

## Empty list-proposals

```
$ archon behaviour list-proposals
No behaviour proposals found.
```

## Empty generate-proposals

```
$ archon behaviour generate-proposals
No proposals generated (thresholds not met).
Scanned 0 learning event(s).
```

## Show non-existent ID

```
$ archon behaviour show nonexistent
No proposal, version, or event found with ID: nonexistent
```

## History for manifest kind with no versions

```
$ archon behaviour history RetrievalProfile
No version history found for manifest kind: RetrievalProfile
```

## Verification

- All empty-DB paths return clean user-facing messages, not raw errors
- `show` attempts all three types (proposal, version, event) before reporting not found
- `generate-proposals` reports scan count even when no thresholds are met
- No stack traces, panics, or CozoDB errors leak to the user

## Date Captured

2026-05-03
