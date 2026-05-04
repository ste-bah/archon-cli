# Transcript 04 — Full Governed Loop (Integration Test)

## Test: `test_full_governed_loop_event_to_apply_to_rollback`

### Step 1: Record outcome signals (3 SourceContradicted events for same source)

Three `SourceContradicted` learning events recorded for `source-loop` in workspace `ws-loop`, each with confidence 0.9.

### Step 2: Generate proposals from events

```
generate_proposals(&events) → 1 proposal
  - Kind: SourceQualityProfile
  - Risk: Low
  - Evidence IDs: 3
  - Status: Pending
```

Proposal persisted to `behaviour_proposals`.

### Step 3: Policy evaluation

```
evaluate_proposal(db, proposal, allow_auto_apply=true, recent_incidents=0)
  → Decision: AutoApplied
  → Rule: low_risk_auto_apply
```

### Step 4: Apply decision

```
apply_decision(db, proposal_id, AutoApplied, content={"weight": 0.7})
  → Status: Applied
  → New version created with version_number > 0
  → ManifestApplied learning event logged
```

### Step 5: Rollback

```
rollback_to_version(db, version_id, "ws-loop", "integration test rollback")
  → is_rollback_target: true
  → New version created (rollback creates new version, never mutates)
  → ManifestRolledBack learning event logged
```

### Step 6: Audit trail verification

```
list_all_learning_events() → at least 2 manifest events:
  - ManifestApplied (from apply step)
  - ManifestRolledBack (from rollback step)
  + 3 SourceContradicted (from signal recording)
```

## CLI Equivalent Commands (manual replay)

```sh
# Seed 3 contradiction events (via test infrastructure)
# Then:
archon behaviour generate-proposals    # → 1 proposal generated
archon behaviour list-proposals        # → 1 Pending proposal
archon behaviour apply bp-<id>        # → Auto-applied
archon behaviour history SourceQualityProfile  # → 2 versions (original + rollback)
archon behaviour status                # → 1 applied, 1 rolled back
```

## Verification

- Full end-to-end: event → proposal → policy → apply → rollback
- All 25 archon-learning tests pass (including this one)
- Audit trail complete: events linked to proposals, proposals linked to versions
- Rollback creates new version (immutable), never overwrites
- Policy decisions are explainable (rule_name, reason recorded)

## Date Captured

2026-05-03
