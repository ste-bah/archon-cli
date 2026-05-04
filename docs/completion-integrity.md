# Completion Integrity

Completion integrity prevents unsupported "done", "tests pass", "fixed",
"indexed", or "cited" claims from silently becoming learning data. It extracts
completion-sensitive claims, resolves independent evidence, runs verification
gates, records false-completion incidents, and updates agent/model trust scores.

## CLI

Current `archon completion --help` surface:

| Command | Purpose |
|---|---|
| `inspect <run-id>` | Inspect claims, evidence, and report state for a run |
| `claims <run-id>` | List completion-sensitive claims for a run |
| `evidence <run-id>` | List evidence records for a run |
| `incidents` | List false-completion incidents |
| `verify <run-id>` | Read output text from stdin, run gates, return pass/fail exit code |
| `trust` | Show persisted agent/model trust scores |

`verify` accepts:

| Flag | Meaning |
|---|---|
| `--task-type <type>` | Task type for trust grouping and gate context |
| `--agent <key>` | Agent key responsible for the output |
| `--model <name>` | Model responsible for the output |
| `--workspace-id <id>` | Workspace dimension for trust grouping |
| `--require-claims` | Fail if no prior claims are found for the run |

`trust` accepts:

| Flag | Meaning |
|---|---|
| `--agent <key>` | Filter to one agent key |
| `--model <name>` | Filter to one model |

## Source of truth

The completion subsystem stores:

| Relation | Meaning |
|---|---|
| `completion_claims` | extracted claims, kind, text, verification status |
| `completion_evidence` | evidence records read by gates |
| `verification_gate_results` | per-gate pass/fail state |
| `completion_reports` | calibrated report summary |
| `false_completion_incidents` | persisted failure incidents |
| `completion_run_contexts` | run id to workspace/agent/model dimensions |
| `agent_model_trust_scores` | computed trust scores |

Trust scoring is documented in `crates/archon-completion/src/trust.rs`. It is
grouped by `workspace_id + agent_key + model + task_type`, uses verified claim
counts and severity-weighted false-completion counts, and is recomputed after
each verification run.

## Full State Verification

```bash
printf 'All tests pass. Implementation is done.' | \
  archon completion verify run-1 \
    --task-type coding \
    --agent verifier-agent \
    --model claude-sonnet \
    --workspace-id my-workspace

archon completion inspect run-1
archon completion incidents
archon completion trust --agent verifier-agent --model claude-sonnet
```

The success criterion is not the `verify` return code alone. The independent
read must show persisted claims/evidence/incidents and a trust row with the
expected workspace, agent, model, and task type.
