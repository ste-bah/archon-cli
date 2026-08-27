use std::sync::atomic::{AtomicUsize, Ordering};

use archon_core::config::GateMode;

use super::*;

fn finding(text: &str) -> GateFinding {
    GateFinding::new(
        GateId::WorkflowLintTaskFile,
        text,
        "TASK-X-010",
        Some("tasks/PRD-X/TASK-X-010.md".into()),
    )
}

#[test]
fn off_skips_evaluation_and_returns_an_explicit_non_pass() {
    let temp = tempfile::tempdir().unwrap();
    let calls = AtomicUsize::new(0);
    let mut output = run_sync_gate(
        temp.path(),
        GateMode::Off,
        GateId::WorkflowLintTaskFile,
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            panic!("off must not evaluate")
        },
    )
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(output.report(), OFF_MESSAGE);
    assert!(output.diagnostics().is_empty());
    assert!(output.take_publication_permit().is_none());
    assert!(!temp.path().join(".archon").exists());
}

#[test]
fn observe_records_adjudicable_verbatim_findings_and_does_not_block() {
    let temp = tempfile::tempdir().unwrap();
    let exact = "TASK-X-010: replace `true` with a predicate that can fail";
    let mut output = run_sync_gate(
        temp.path(),
        GateMode::Observe,
        GateId::WorkflowLintTaskFile,
        || Ok(GateEvaluation::new("report\n", vec![finding(exact)])),
    )
    .unwrap();

    assert_eq!(output.report(), "report\n");
    assert_eq!(output.diagnostics(), &[format!("[shadow] {exact}")]);
    assert!(output.take_publication_permit().is_some());
    let body = std::fs::read_to_string(shadow_log_path(temp.path())).unwrap();
    let row: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
    assert_eq!(row["schema_version"], "workflow-gate-shadow-v1");
    assert_eq!(row["gate_id"], "workflow_lint.task_file");
    assert_eq!(row["finding"], exact);
    assert_eq!(row["subject"], "TASK-X-010");
    assert_eq!(row["source_path"], "tasks/PRD-X/TASK-X-010.md");
    assert_eq!(row["mode"], "observe");
    assert_eq!(row["binary_commit"], env!("ARCHON_GIT_HASH"));
    assert!(chrono::DateTime::parse_from_rfc3339(row["timestamp"].as_str().unwrap()).is_ok());
}

#[test]
fn enforce_blocks_with_the_verbatim_actionable_finding() {
    let temp = tempfile::tempdir().unwrap();
    let exact = "TASK-X-010: add a verifier that can exit non-zero";
    let output = run_sync_gate(
        temp.path(),
        GateMode::Enforce,
        GateId::WorkflowLintTaskFile,
        || Ok(GateEvaluation::new("report\n", vec![finding(exact)])),
    )
    .unwrap();
    assert_eq!(output.report(), "report\n");
    let error = output.blocking_error.expect("enforce blocker");
    assert!(error.contains(exact), "{error}");
    assert!(!temp.path().join(".archon").exists());
}

#[test]
fn observe_clean_creates_no_shadow_log_and_log_failure_is_operational() {
    let clean = tempfile::tempdir().unwrap();
    run_sync_gate(
        clean.path(),
        GateMode::Observe,
        GateId::RequirementsTrace,
        || Ok(GateEvaluation::new("clean\n", Vec::new())),
    )
    .unwrap();
    assert!(!shadow_log_path(clean.path()).exists());

    let broken = tempfile::tempdir().unwrap();
    std::fs::write(broken.path().join(".archon"), "not a directory").unwrap();
    let error = run_sync_gate(
        broken.path(),
        GateMode::Observe,
        GateId::WorkflowLintTaskFile,
        || Ok(GateEvaluation::new("report\n", vec![finding("exact")])),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("shadow log"), "{error}");
}

#[test]
fn canonical_ids_become_shadow_subjects_when_present() {
    assert_eq!(
        finding_subject(
            "task 'TASK-X-010' cites unknown obligation 'REQ-X-999'",
            "fallback"
        ),
        "TASK-X-010"
    );
    assert_eq!(
        finding_subject("PRD obligation 'REQ-X-001' is unclaimed", "fallback"),
        "REQ-X-001"
    );
    assert_eq!(
        finding_subject("no canonical id here", "task directory x"),
        "task directory x"
    );
}

#[test]
fn operational_failure_preserves_report_and_blocks_in_observe_without_shadow_logging() {
    let temp = tempfile::tempdir().unwrap();
    let disposition = run_sync_gate(
        temp.path(),
        GateMode::Observe,
        GateId::RequirementsTrace,
        || {
            Ok(GateEvaluation::new("partial report\n", Vec::new())
                .with_operational_error("TASK-X-010.md is malformed; repair its YAML"))
        },
    )
    .unwrap();

    assert_eq!(disposition.report(), "partial report\n");
    assert!(
        disposition
            .require_allowed()
            .unwrap_err()
            .to_string()
            .contains("TASK-X-010.md is malformed")
    );
    assert!(disposition.diagnostics().is_empty());
    assert!(!shadow_log_path(temp.path()).exists());
}

#[test]
fn shadow_records_are_one_complete_write_each_and_short_writes_fail() {
    #[derive(Default)]
    struct RecordingWriter {
        writes: Vec<Vec<u8>>,
        short: bool,
    }
    impl std::io::Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.push(buf.to_vec());
            Ok(if self.short {
                buf.len().saturating_sub(1)
            } else {
                buf.len()
            })
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let records = vec!["{\"row\":1}".to_string(), "{\"row\":2}".to_string()];
    let mut writer = RecordingWriter::default();
    append_serialized_records(&mut writer, &records).unwrap();
    assert_eq!(writer.writes.len(), 2);
    assert_eq!(writer.writes[0], b"{\"row\":1}\n");
    assert_eq!(writer.writes[1], b"{\"row\":2}\n");

    let mut short = RecordingWriter {
        short: true,
        ..Default::default()
    };
    let error = append_serialized_records(&mut short, &records)
        .unwrap_err()
        .to_string();
    assert!(error.contains("short write"), "{error}");
    assert_eq!(
        short.writes.len(),
        1,
        "retry must not duplicate earlier rows"
    );
}
