//! Top-level mirrors of the typed aggregate's evidence arrays in the v3
//! envelope (Issue-19).
//!
//! The deduplicated view spreads `result.data` at the top level and carries
//! the typed aggregate under `result`, so `files_changed`, `commands_run`,
//! `evidence`, `residual_gaps` and `artifacts` lived ONLY at `result.*`. Every
//! authored script so far read them at the top level: live (wf-719ff3b0) the
//! script's `isAccepted(env)` required `env.files_changed`/`env.commands_run`
//! to be non-empty, the verifier's envelope carried 21 commands under
//! `result.commands_run` and none at the top, the predicate answered false on
//! an accepted verify, and a remediation round was launched after every
//! accepted verification. The mirrors make that read — and the prelude's own
//! `usable(env)` fallback, which reads the top level when `env.result` is
//! absent — see the same evidence the typed aggregate carries.
//!
//! Size: every mirrored byte is a second copy of something under `result`, and
//! an authored script's remediation prompt stringifies the whole envelope, so
//! the three arrays with free text or per-item bulk are mirrored as COMPACT
//! projections rather than full records: `files_changed` as paths,
//! `commands_run` as `{command, status}` (no `output_summary`, the one
//! unbounded string), `residual_gaps` as `{id, severity}`. `evidence` and
//! `artifacts` are mirrored whole; their records are already short. Measured on
//! the live wf-719ff3b0 verify envelope (21 commands, 18 evidence entries,
//! `result_view_tests::the_mirrors_add_a_bounded_share_to_the_live_verify_envelope`):
//! the deduplicated view is 81,530 bytes without mirrors; the compact mirrors
//! add 16,003 (+19.6%), a full-record copy would add 19,935 (+24.4%). Most of
//! the compact copy is the 292-char average `command` line a verifier runs
//! plus the evidence summaries. The compat view of the same result is 225,164
//! bytes. The full records stay under `result.*`; the reference tells authors
//! that is where to read them for reporting. `items` and `outcomes` are NOT
//! mirrored: they are the top-level arrays already (Issue-8 keeps each branch
//! result single-copy).
//!
//! A `result.data` key of the same name is overwritten by the mirror, exactly
//! as `status` and `summary` already overwrite theirs: the host's typed field
//! is the authoritative one, and the raw data copy stays at `result.data.*`.
//! The compat shape is unchanged.

use super::*;

/// The top-level keys the deduplicated view mirrors from the typed aggregate.
pub const MIRRORED_RESULT_KEYS: [&str; 5] = [
    "files_changed",
    "commands_run",
    "evidence",
    "residual_gaps",
    "artifacts",
];

/// Insert the mirrors into `view` (the deduplicated envelope under
/// construction) from the typed `result`.
pub(super) fn mirror_typed_arrays(
    view: &mut serde_json::Map<String, serde_json::Value>,
    result: &WorkflowV2Result,
) -> WorkflowResult<()> {
    let files_changed = result
        .files_changed
        .iter()
        .map(|file| serde_json::Value::String(file.path.clone()))
        .collect::<Vec<_>>();
    let commands_run = result
        .commands_run
        .iter()
        .map(|command| {
            Ok(serde_json::json!({
                "command": command.command,
                "status": serde_json::to_value(command.status)?,
            }))
        })
        .collect::<WorkflowResult<Vec<_>>>()?;
    let residual_gaps = result
        .residual_gaps
        .iter()
        .map(|gap| serde_json::json!({ "id": gap.id, "severity": gap.severity }))
        .collect::<Vec<_>>();
    view.insert(
        "files_changed".to_string(),
        serde_json::Value::Array(files_changed),
    );
    view.insert(
        "commands_run".to_string(),
        serde_json::Value::Array(commands_run),
    );
    view.insert(
        "evidence".to_string(),
        serde_json::to_value(&result.evidence)?,
    );
    view.insert(
        "residual_gaps".to_string(),
        serde_json::Value::Array(residual_gaps),
    );
    view.insert(
        "artifacts".to_string(),
        serde_json::to_value(&result.artifacts)?,
    );
    Ok(())
}
