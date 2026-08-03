use super::*;
use archon_topology::ir::WriteTarget;
use archon_topology::trace::{TraceKind, TraceRecord};

#[test]
fn commands_run_and_tests_run_are_both_verifier_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("report.json");
    std::fs::write(
        &path,
        r#"{
          "commands_run": [
            {"kind":"test","command":"cargo test -p x","status":"succeeded","exit_code":0,
             "output_summary":"ok"},
            {"kind":"build","command":"cargo build","status":"failed","exit_code":101,
             "output_summary":"boom"}
          ],
          "tests_run": [
            {"kind":"test","command":"cargo test -p y","status":"succeeded","exit_code":0,
             "output_summary":"ok"}
          ],
          "status": "completed",
          "unknown_future_field": 7
        }"#,
    )
    .expect("write");

    let commands = load_commands(&path).expect("parse");
    assert_eq!(commands.len(), 3);
    assert!(commands[0].passed());
    assert!(!commands[1].passed());
    assert_eq!(commands[1].exit_code, Some(101));
    assert_eq!(commands[2].command, "cargo test -p y");
}

#[test]
fn a_report_with_no_commands_parses_as_no_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("report.json");
    std::fs::write(&path, r#"{"status":"completed"}"#).expect("write");
    assert!(load_commands(&path).expect("parse").is_empty());
}

#[test]
fn a_missing_evidence_file_is_an_error_naming_the_path() {
    let err = load_commands(Path::new("does/not/exist.json")).expect_err("refused");
    assert!(err.to_string().contains("does/not/exist.json"), "{err}");
}

fn write_trace(root: &Path, graph_id: &str, records: &[TraceRecord]) {
    let paths = TopologyPaths::for_project(root);
    let dir = paths.trace_jsonl(graph_id);
    std::fs::create_dir_all(dir.parent().expect("parent")).expect("mkdir");
    let body: String = records
        .iter()
        .map(|r| format!("{}\n", serde_json::to_string(r).expect("serialize")))
        .collect();
    std::fs::write(&dir, body).expect("write trace");
}

fn record(kind: TraceKind, node: &str, path: &str) -> TraceRecord {
    TraceRecord::new("2026-08-03T00:00:00Z", "g1", kind)
        .with_node(node)
        .with_reads(vec![WriteTarget::Path(path.to_string())])
        .with_writes(vec![WriteTarget::Path(path.to_string())])
}

#[test]
fn only_file_read_records_count_as_reads() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_trace(
        dir.path(),
        "g1",
        &[
            record(TraceKind::FileRead, "TASK-A", "src/a.rs"),
            // A written file is not evidence a verifier exercised it; folding
            // writes in would let a task promote its own output.
            record(TraceKind::FileWritten, "TASK-A", "src/written.rs"),
            record(TraceKind::ToolAttempt, "TASK-A", "src/attempt.rs"),
        ],
    );
    let reads = load_reads(dir.path(), "g1").expect("read");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].node_id, "TASK-A");
    assert_eq!(reads[0].file_path, "src/a.rs");
}

#[test]
fn windows_separators_and_dot_prefixes_normalise_to_the_anchor_form() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_trace(
        dir.path(),
        "g1",
        &[
            record(TraceKind::FileRead, "TASK-A", r".\src\a.rs"),
            record(TraceKind::FileRead, "TASK-A", "./src/b.rs"),
        ],
    );
    let paths: Vec<String> = load_reads(dir.path(), "g1")
        .expect("read")
        .into_iter()
        .map(|r| r.file_path)
        .collect();
    assert_eq!(paths, ["src/a.rs", "src/b.rs"]);
}

#[test]
fn a_graph_that_was_never_run_has_no_reads_rather_than_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(
        load_reads(dir.path(), "never-ran")
            .expect("empty")
            .is_empty()
    );
}
