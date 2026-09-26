//! Issue-116: which sidecar records are the host's evidence that a tool ran.
use super::*;
use serde_json::json;

fn write(path: &Path, records: &[Value]) {
    let text: String = records.iter().map(|r| format!("{r}\n")).collect();
    std::fs::write(path, text).unwrap();
}

fn call(tool: &str, status: &str) -> Value {
    json!({"kind": "tool_call", "call": 1, "tool": tool, "head": "", "status": status})
}

#[test]
fn only_calls_that_ran_after_the_start_are_observed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sidecar.jsonl");
    write(&path, &[call("mcp__a__before", "ok")]);
    let log = HostToolLog::from_now(path.clone());
    let mut records = vec![call("mcp__a__before", "ok")];
    records.extend([
        call("mcp__a__ok", "ok"),
        call("mcp__a__failed", "error: connection refused"),
        call("mcp__a__refused", "refused: read budget exhausted"),
        call("Bash", "exit 0"),
        json!({"kind": "refusal", "call": 2, "tool": "mcp__a__refusal", "reason": "no"}),
        json!({"path": "src/lib.rs", "offset": 0, "limit": 10, "call": 3, "hash": "h"}),
    ]);
    // Appended after the start, as the guard appends.
    write(&path, &records);
    assert_eq!(
        log.executed_tools(),
        vec!["mcp__a__ok", "mcp__a__failed", "Bash"]
    );
}

#[test]
fn a_missing_or_shrunk_sidecar_observes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sidecar.jsonl");
    assert!(
        HostToolLog::from_now(path.clone())
            .executed_tools()
            .is_empty()
    );
    write(&path, &[call("mcp__a__x", "ok"), call("mcp__a__y", "ok")]);
    let log = HostToolLog::from_now(path.clone());
    write(&path, &[call("mcp__a__z", "ok")]);
    assert!(log.executed_tools().is_empty());
}

#[tokio::test]
async fn observed_tools_reads_the_scoped_log_only() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sidecar.jsonl");
    let log = HostToolLog::from_now(path.clone());
    write(&path, &[call("mcp__a__x", "ok")]);
    assert!(observed_tools().is_empty());
    let seen = scope(log, async { observed_tools() }).await;
    assert_eq!(seen, vec!["mcp__a__x"]);
}
