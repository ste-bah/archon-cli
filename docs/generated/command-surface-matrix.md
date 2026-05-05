# Command surface matrix

Generated from `src/command/surface_matrix.rs`. Update the code-owned matrix and regenerate this file when command surfaces change.

| CLI surface | Slash primary | TUI surface | Status | Source of truth | Notes |
|---|---|---|---|---|---|
| `archon auth ...` | `/auth` | CLI mirror | DONE | `src/command/registry.rs + src/cli_args.rs` | Provider login/status/logout is available from shell and TUI. |
| `archon chat --provider <id> <prompt>` | `/chat` | CLI mirror | DONE | `src/command/registry.rs + src/command/chat.rs` | One-shot provider chat is mirrored into the TUI. |
| `archon providers ...` | `/providers` | Direct slash handler | DONE | `src/command/providers.rs` | Provider list, capabilities, and doctor are available in both surfaces. |
| `archon docs ...` | `/docs` | Evidence browser + CLI mirror | DONE | `src/command/docs.rs + src/command/evidence_view.rs` | Document ingest/search/inspect routes through persisted document state. |
| `archon kb ...` | `/kb` | CLI mirror | DONE | `src/command/registry.rs` | Knowledge claims, entities, relations, contradictions, and search are mirrored. |
| `archon prov ...` | `/prov` | CLI mirror | DONE | `src/command/registry.rs` | Trace, export, and verify run through the same provenance store. |
| `archon gametheory ...` | `/gametheory` | Direct slash handler | DONE | `src/command/gametheory_slash.rs` | Run, classify-only, status, inspect, replay, agents, and specimens are exposed. |
| `archon completion ...` | `/completion` | CLI mirror | DONE | `src/command/registry.rs` | Completion integrity inspection and trust surfaces are mirrored. |
| `archon behaviour ...` | `/behaviour` | CLI mirror | DONE | `src/command/registry.rs` | Governed-learning events, proposals, approvals, rollback, and status are mirrored. |
| `archon meaning ...` | `/meaning` | CLI mirror | DONE | `src/command/registry.rs` | Meaning samples, contrastive pairs, triplets, and export are mirrored. |
| `archon constellation ...` | `/constellation` | CLI mirror | DONE | `src/command/registry.rs` | Centroid build, score, drift, and list commands are mirrored. |
| `archon pipeline ...` | `/pipeline` | CLI mirror | DONE | `src/command/registry.rs` | Pipeline run/status/resume/list/abort/cancel are mirrored. |
| `archon pipeline code <task>` | `/archon-code` | Pipeline primary | DONE | `src/command/archon_code.rs` | The coding pipeline has a first-class TUI slash primary. |
| `archon pipeline research <topic>` | `/archon-research` | Pipeline primary | DONE | `src/command/archon_research.rs` | The research pipeline has a first-class TUI slash primary. |
| `archon agent-list/search/info` | `/agent` | Agent umbrella | DONE | `src/command/agent_slash.rs` | Agent list, info, and run are grouped under /agent. |
| `archon run-agent-async ...` | `/run-agent` | Custom-agent launcher | PARTIAL | `src/command/run_agent.rs + src/command/task.rs` | Launch is slash-native; async task status/result/cancel/list/events use /tasks and shell commands. |
| `archon task-status/result/cancel/list/events` | `/tasks` | Task browser | PARTIAL | `src/command/task.rs` | /tasks covers listing and task visibility; individual shell subcommands remain richer. |
| `archon plugin ...` | `/plugin` | Plugin umbrella | PARTIAL | `src/command/plugin_slash.rs` | List/info are live; enable/disable/install/reload emit guidance until persistent plugin state exists. |
| `archon team ...` | - | Not yet mirrored | SHELL_ONLY | `src/cli_args.rs + src/command/team.rs` | Team execution is shell-only until a /team handler is wired. |
| `archon serve/remote/web/ide-stdio` | - | Host process control | SHELL_ONLY | `src/cli_args.rs` | Process-mode commands intentionally remain shell-only. |
| `archon metrics/update` | - | Operations shell | SHELL_ONLY | `src/cli_args.rs` | Operational commands are shell-first; TUI mirrors can be added if product need appears. |
