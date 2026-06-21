use std::path::Path;

use serde_json::{Value, json};

pub(crate) fn guidance(
    _project: &Path,
    artifact_path: &str,
    resolved_path: &str,
    invalid_reason: Option<&str>,
) -> Value {
    json!({
        "artifact_path": artifact_path,
        "resolved_path": resolved_path,
        "repair_mode": "materialize_or_repair_source_then_explain",
        "must_attempt_generation": true,
        "candidate_commands": [
            {
                "command": "bash -lc 'source ./profile 2>/dev/null || true; if [ -x ./archon ]; then ./archon --help; elif command -v archon >/dev/null 2>&1; then archon --help; else true; fi'",
                "purpose": "load project environment and discover available Archon materialization commands"
            },
            {
                "command": "bash -lc 'source ./profile 2>/dev/null || true; rg -n \"required_artifacts|artifact|backtest|dataset|data-lake|generate|materialize\" . -g \"*.md\" -g \"*.toml\" -g \"*.rs\"'",
                "purpose": "discover the concrete project command or source path that should materialize this artifact"
            }
        ],
        "command_discovery_required_if_blocked": true,
        "blocked_response_requires": [
            "status=blocked",
            "artifact_path or resolved_path",
            "reason or missing_evidence",
            "commands_run, attempted_commands, generation_attempts, or command_discovery",
        ],
        "success_requires": [
            "the concrete artifact exists at resolved_path",
            "the artifact is not placeholder, synthetic, or empty evidence",
            "the response names commands run or source changes made to materialize it",
        ],
        "invalid_reason": invalid_reason,
    })
}
