//! Captured check output: the inline tail a round record keeps, and the
//! full output written beside it; and the frozen identity each failing
//! check is reported with.

use std::path::Path;

use archon_workflow::WorkflowV2Result;
use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::failure_evidence::failure_evidence;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceCheck, AcceptanceContract, AcceptanceCriterion, TrustedCwd,
};
use archon_workflow::v2::acceptance_stage::{AcceptanceExecutionRecordV1, AcceptanceRoundRecordV1};

/// Bytes of stdout/stderr kept inline in the round record, and handed to a
/// remediation agent verbatim; the full captured output (bounded by the
/// site's output limit) is written beside it.
const OUTPUT_TAIL_BYTES: usize = 4000;

/// A stream's failure evidence: its end and every line stating a failure,
/// bounded (`archon_workflow::failure_evidence`). Never a bare byte tail: a
/// long `cargo run` prints its one `Error:` line after pages of warnings.
pub(super) fn tail(bytes: &[u8]) -> String {
    failure_evidence(bytes, OUTPUT_TAIL_BYTES)
}

/// The short form a command record's `output_summary` carries.
pub(super) fn brief(evidence: &str) -> String {
    failure_evidence(evidence.as_bytes(), 400)
}

pub(super) fn write_output_files(dir: &Path, result: &CheckResult) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let safe: String = result
        .acceptance_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let _ = std::fs::write(dir.join(format!("{safe}.stdout")), &result.stdout);
    let _ = std::fs::write(dir.join(format!("{safe}.stderr")), &result.stderr);
}

/// The contract this round executed, as the stage held it in memory (after
/// any in-round repair), and its content digest.
pub(super) struct Frozen {
    pub(super) contract: AcceptanceContract,
    pub(super) digest: String,
}

/// Batch I2: each failing check in the reply carries `frozen_check`, the
/// text a remediation agent is shown verbatim: the absolute path of the
/// contract the harness executed (the run's recorded task root, never a
/// guess), its digest, how and where the harness ran it, and the check's
/// exact entry, taken from the contract the round ran, never re-read. Any
/// other copy of the contract is named as not authoritative.
pub(super) fn with_frozen_identity(
    record: &AcceptanceRoundRecordV1,
    frozen: Option<&Frozen>,
    mut result: WorkflowV2Result,
) -> WorkflowV2Result {
    let (Some(execution), Some(frozen)) = (record.execution.as_ref(), frozen) else {
        return result;
    };
    let path = Path::new(&execution.task_root).join(ACCEPTANCE_CONTRACT_FILE);
    let path = path.canonicalize().unwrap_or(path);
    result.data["contract_path"] = path.display().to_string().into();
    let failing = result
        .data
        .get_mut("failing")
        .and_then(|f| f.as_array_mut());
    for entry in failing.into_iter().flatten() {
        let id = entry["check_id"].as_str().unwrap_or_default().to_string();
        let found = (frozen.contract.acceptance.iter())
            .chain(&frozen.contract.supplementary)
            .find(|criterion| criterion.id == id);
        entry["frozen_check"] = match found {
            Some(criterion) => frozen_check(&path, &frozen.digest, execution, criterion),
            None => format!(
                "FROZEN CHECK {id}: not in the contract this round ran ({}); do not look for it in any other acceptance contract copy",
                path.display()
            ),
        }
        .into();
    }
    result
}

fn frozen_check(
    path: &Path,
    digest: &str,
    execution: &AcceptanceExecutionRecordV1,
    criterion: &AcceptanceCriterion,
) -> String {
    let at = execution
        .source_commit
        .as_deref()
        .map_or(String::new(), |commit| format!(" at commit {commit}"));
    let how = if execution.mode == "scratch" {
        format!(
            "in a hermetic scratch: a fresh clone of the repository{at} (committed state only: an uncommitted edit is not in it), the project's input data copied beside it, the command fed to `sh -s`, a private HOME, TMPDIR, CARGO_HOME and CARGO_TARGET_DIR, and only these host settings: {}",
            execution.environment
        )
    } else {
        format!(
            "directly on the live tree (the target repository checkout{at}, uncommitted and ignored files included; a project-root check in the project root): {}",
            execution.environment
        )
    };
    let entry = match &criterion.check {
        AcceptanceCheck::Command { command, cwd } => format!(
            "kind command, run from the {} -- reproduce it from YOUR checkout, never by running in the live project or repository path; the exact command, verbatim:\n{command}",
            match cwd {
                TrustedCwd::RepoRoot => "repository root",
                TrustedCwd::ProjectRoot =>
                    "project root (in your worktree: its root, where the project's input data is seeded at the same relative paths)",
            }
        ),
        other => format!(
            "the exact entry, verbatim: {}",
            serde_json::to_string(other).unwrap_or_default()
        ),
    };
    format!(
        "FROZEN CHECK {id} -- exactly what the harness runs. The only authoritative acceptance contract is {} (content digest {digest}); any other copy or draft of the acceptance contract, whatever its name, in the repository, your worktree or the project is NOT authoritative: never read, run or cite it as this check. The harness ran it {how}. A run under other conditions (another environment, another binary, another tree state) can pass where the harness fails: reproduce the harness's conditions before concluding it passes. {id}: {entry}",
        path.display(),
        id = criterion.id,
    )
}
