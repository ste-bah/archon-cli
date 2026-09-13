//! The worked example's wave extraction against the envelope a v3 script is
//! REALLY handed.
//!
//! The example used to read `batch.data.outcomes` / `batch.data.items`. The
//! host spreads a fan-out's `data` at the TOP level of the script envelope
//! (`result_view_json_shaped`), so there is no `batch.data` key: both reads
//! were always `[]`, every task's implementation envelope became the
//! `no outcome returned for <id> in its wave batch` stub, and every task was
//! remediated unconditionally — seen verbatim in a live remediation prompt.
//! The dry-run rehearsal never caught it because its stub DID emit a `data`
//! wrapper. These tests run the example's own extraction lines, cut from the
//! reference text rather than copied, against an envelope the live view
//! function rendered, and against the rehearsal stub.

use crate::v2::call_data::result_from_fanout_report;
use crate::v2::script::{ScriptEnvelopeShape, result_view_json_shaped};
use crate::{
    WorkflowV2BranchOutcome, WorkflowV2FanoutReport, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2Result, WorkflowV2WriteMode,
};

const TASK: &str = "TASK-X-001";
const MISSING: &str = "TASK-X-404";

/// The example's wave-extraction lines, cut from the reference: from the
/// `outcomesOf(batch)` read to the end of the `for (const id of wave)` loop.
fn example_extraction() -> String {
    let reference = super::V3_PRIMITIVE_REFERENCE;
    let start = reference
        .find("    const branches = outcomesOf(batch)")
        .expect("the example reads the wave batch through outcomesOf");
    let body = &reference[start..];
    let loop_end = body
        .find("\n    }\n")
        .expect("the wave loop closes at the example's indentation");
    body[..loop_end + "\n    }".len()].to_string()
}

/// Pull one named arrow-function definition out of the prelude by name.
fn prelude_fn(name: &str) -> String {
    let prelude = super::super::V3_PRIMITIVES_JS;
    let marker = format!("  const {name} = ");
    let start = prelude
        .find(&marker)
        .unwrap_or_else(|| panic!("prelude must define {name}"));
    let end = start
        + prelude[start..]
            .find("\n  };")
            .unwrap_or_else(|| panic!("{name} must end with a closing arrow body"))
        + 5;
    prelude[start..end].to_string()
}

/// Run the example extraction over `envelope` for a wave naming the real task
/// and one the batch never returned; report what `implOf` holds for each,
/// plus what the old `batch.data.outcomes` read would have seen.
fn run_extraction(envelope: &str) -> serde_json::Value {
    let driver = format!(
        r#"{outcomes_of}
const batch = {envelope};
const wave = ['{TASK}', '{MISSING}'];
const implOf = {{}};
{extraction}
console.log(JSON.stringify({{
  present: implOf['{TASK}'],
  missing: implOf['{MISSING}'],
  legacyRead: ((batch && batch.data && batch.data.outcomes) || []).length,
}}));
"#,
        outcomes_of = prelude_fn("outcomesOf"),
        extraction = example_extraction(),
    );
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("extract.mjs");
    std::fs::write(&path, driver).expect("write driver");
    let out = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available");
    assert!(
        out.status.success(),
        "driver failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("driver prints JSON")
}

fn write_wave_call() -> WorkflowV2HostCall {
    let mut call = WorkflowV2HostCall {
        id: "implement-wave-1".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: Default::default(),
    };
    call.options.item_kind = Some("implementation".to_string());
    call
}

/// One accepted implementation branch that claims `TASK`, with the work
/// evidence a live write branch carries.
fn accepted_branch() -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result::accepted("implemented TASK-X-001");
    result.files_changed = vec![crate::WorkflowV2FileRecord::new("src/module.ext")];
    result.commands_run = vec![crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Test,
        command: "cargo test -p module".to_string(),
        status: crate::WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "1 passed".to_string(),
    }];
    result.data = serde_json::json!({
        "item_id": "implement-task-x-001",
        "canonical_task_ids": [TASK],
    });
    WorkflowV2BranchOutcome {
        item_id: "implement-task-x-001".to_string(),
        role: "coder".to_string(),
        status: result.status,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

/// The envelope the live host hands a v3 script for a one-item write wave.
fn live_envelope() -> String {
    let normalized = result_from_fanout_report(
        &write_wave_call(),
        WorkflowV2FanoutReport {
            outcomes: vec![accepted_branch()],
            max_parallelism: 1,
            peak_parallelism: 1,
            cancelled: false,
        },
    );
    result_view_json_shaped(&normalized.result, ScriptEnvelopeShape::Deduped).expect("view")
}

#[test]
fn the_example_reads_each_task_from_the_live_envelope() {
    let envelope = live_envelope();
    let view: serde_json::Value = serde_json::from_str(&envelope).unwrap();
    // The shape the example must read: top-level arrays, no `data` wrapper.
    assert!(view.get("data").is_none(), "{envelope}");
    assert_eq!(
        view["outcomes"][0]["canonical_task_ids"][0], TASK,
        "{envelope}"
    );
    let item_status = view["items"][0]["status"].clone();
    assert_eq!(item_status, "accepted", "{envelope}");

    let got = run_extraction(&envelope);
    assert_eq!(
        got["legacyRead"], 0,
        "the old `batch.data.outcomes` read sees nothing on the live envelope: {got}"
    );
    assert_eq!(
        got["present"]["status"], item_status,
        "the implemented task must carry its branch's own status, not the missing-outcome stub: {got}"
    );
    assert_eq!(got["present"]["canonical_task_ids"][0], TASK, "{got}");
    assert!(
        got["present"]["files_changed"]
            .as_array()
            .is_some_and(|f| !f.is_empty()),
        "the implementation envelope must carry the branch's work evidence for remediation: {got}"
    );
    // A task the wave never returned still gets the failed stub the
    // remediation loop acts on.
    assert_eq!(got["missing"]["status"], "failed", "{got}");
    assert!(
        got["missing"]["summary"]
            .as_str()
            .is_some_and(|s| s.contains(MISSING)),
        "{got}"
    );
}

/// The rehearsal answers in the live shape, so a shape drift in the example
/// fails the pre-flight instead of every live run.
#[test]
fn the_rehearsal_stub_has_the_live_envelope_shape() {
    let mut call = write_wave_call();
    call.id = "agents-1".to_string();
    let payload = serde_json::json!({
        "id": "agents-1",
        "source": [{ "canonical_task_ids": [TASK] }],
        "options": {},
    })
    .to_string();
    let stub =
        super::super::dry_run_b::dry_run_stub_result(&call, &payload, ScriptEnvelopeShape::Deduped)
            .expect("stub");
    let view: serde_json::Value = serde_json::from_str(&stub).unwrap();
    let live: serde_json::Value = serde_json::from_str(&live_envelope()).unwrap();
    assert!(
        view.get("data").is_none(),
        "the live view has no `data` wrapper: {stub}"
    );
    for key in ["items", "outcomes", "status", "summary", "result"] {
        assert!(
            view.get(key).is_some() && live.get(key).is_some(),
            "{key}: {stub}"
        );
    }
    assert!(view["result"]["data"].get("outcomes").is_none(), "{stub}");
    assert!(view["result"]["data"].get("items").is_none(), "{stub}");
    assert_eq!(view["outcomes"][0]["canonical_task_ids"][0], TASK, "{stub}");

    let got = run_extraction(&stub);
    assert_eq!(
        got["legacyRead"], 0,
        "the stub must not carry a `data` wrapper the live view lacks: {got}"
    );
    assert_eq!(got["present"]["status"], "accepted", "{got}");
    assert_eq!(got["missing"]["status"], "failed", "{got}");
}

/// The compat dialect keeps every nested copy live, and so does its rehearsal.
#[test]
fn the_compat_rehearsal_stub_keeps_the_nested_copies() {
    let payload = serde_json::json!({
        "id": "agents-1",
        "source": [{ "canonical_task_ids": [TASK] }],
        "options": {},
    })
    .to_string();
    let stub = super::super::dry_run_b::dry_run_stub_result(
        &write_wave_call(),
        &payload,
        ScriptEnvelopeShape::Compat,
    )
    .expect("stub");
    let view: serde_json::Value = serde_json::from_str(&stub).unwrap();
    assert!(view.get("data").is_none(), "{stub}");
    assert!(view["result"]["data"]["items"].is_array(), "{stub}");
    assert!(view["result"]["data"]["outcomes"].is_array(), "{stub}");
    assert_eq!(view["outcomes"][0]["canonical_task_ids"][0], TASK, "{stub}");
}
