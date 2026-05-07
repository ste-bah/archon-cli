# Governed Learning

Governed learning converts evidence events into reviewable behaviour proposals
instead of letting the system silently rewrite itself. It is the safety layer
between observed outcomes and changed manifests, prompts, policies, thresholds,
or retrieval settings.

> **TUI parity.** Every `archon behaviour <subcommand>` shell form has a `/behaviour <subcommand>` slash equivalent inside the TUI; the live status pane is `/learning-status`. Both forms read and write the same persisted learning-event, proposal, and manifest-version rows. See [CLI and TUI Command Parity](cookbook/real-world-evidence-engine.md#cli-and-tui-command-parity).

## CLI

Current `archon behaviour --help` surface:

| Command | Purpose |
|---|---|
| `list-proposals`, `list` | List behaviour proposals; accepts `--pending` |
| `list-events` | List learning events, optionally filtered by type |
| `show <id>` | Show a proposal, event, or manifest version |
| `apply <proposal-id>` | Auto-apply a pending proposal without human review |
| `history <kind>` | Show version history for a manifest kind |
| `generate-proposals` | Generate proposals from recent learning events |
| `status` | Show learning system status and statistics |
| `approve <proposal-id>` | Human approval path |
| `deny <proposal-id>` | Deny a pending proposal |
| `rollback <version-id>` | Roll back a manifest version; accepts `--reason` |

Related interactive status:

```text
/learning-status
```

## Policy gates

Governed learning is default-deny for auto-apply. Policy controls whether
low-risk updates may apply automatically and whether prompt, blocking-gate,
network, or policy changes require explicit approval.

See [Policy](policy.md) for the TOML format.

## Source of truth

The governed-learning source of truth is persisted learning state:

| State | Meaning |
|---|---|
| learning events | observed outcomes such as false completions or verified completions |
| proposals | proposed behaviour changes derived from evidence |
| manifests | versioned applied state |
| policy decisions | approved, denied, pending, or rolled back decisions |

## Proposal Kinds

The proposal engine currently emits these manifest kinds:

| Kind | Trigger |
|---|---|
| `RetrievalProfile` | Retrieval evidence suggests profile changes |
| `SourceQualityProfile` | Three or more contradictions cluster on one source |
| `AgentRoutingProfile` | Routing evidence suggests profile changes |
| `ConstellationThresholds` | Constellation drift suggests threshold changes |
| `PipelineGates` | Three or more gate failures cluster on one gate |
| `BehaviouralRuleAdjustment` | Three or more user corrections cluster on one behavioural rule within seven days |
| `PromptProfile` | Prompt-profile evidence suggests a reviewed prompt change |
| `PolicyOverride` | Policy evidence suggests an explicit operator-reviewed override |

## Full State Verification

```bash
archon behaviour status
archon behaviour generate-proposals
archon behaviour list --pending
archon behaviour show <proposal-id>
archon behaviour approve <proposal-id>
archon behaviour history <manifest-kind>
```

For an edge-case audit, check no-event state, duplicate proposal generation,
denied policy auto-apply, invalid proposal IDs, and rollback to an older version.
