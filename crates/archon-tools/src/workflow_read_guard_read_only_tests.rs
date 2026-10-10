//! Issue-58: the read-only guard's inspection ceilings. The write-capable
//! guard must not notice any of this.
use super::{
    GuardMode, READ_CEILING_MARKER, REFUSAL_RECORD_KIND, TOOL_CALL_RECORD_KIND, WorkflowReadGuard,
    WorkflowReadGuardSettings,
};
use serde_json::{Value, json};

fn settings(soft: u32, hard: u32) -> WorkflowReadGuardSettings {
    WorkflowReadGuardSettings {
        read_only_soft_call_ceiling: soft,
        read_only_hard_call_ceiling: hard,
        ..Default::default()
    }
}

fn read_only(soft: u32, hard: u32) -> WorkflowReadGuard {
    WorkflowReadGuard::shell_only(&settings(soft, hard))
}

fn read_input() -> Value {
    json!({"file_path": "/repo/src/lib.rs"})
}

fn bash_input(command: &str) -> Value {
    json!({"command": command})
}

/// One admitted inspection call: the verdict and the note its result gets.
fn inspect(
    guard: &WorkflowReadGuard,
    tool: &str,
    input: &Value,
) -> (Option<String>, Option<String>) {
    let verdict = guard.before_tool(tool, input);
    let note = if verdict.is_none() {
        guard.result_note(tool, input)
    } else {
        None
    };
    (verdict, note)
}

#[test]
fn every_inspection_shape_counts_and_nothing_else_does() {
    let guard = read_only(0, 4);
    for (tool, input) in [
        ("Read", read_input()),
        ("Grep", json!({"pattern": "fn main", "path": "/repo"})),
        ("Glob", json!({"pattern": "**/*.rs"})),
        ("read-own-evidence", json!({"offset": 0, "limit": 3})),
    ] {
        assert_eq!(guard.before_tool(tool, &input), None, "{tool}");
    }
    assert_eq!(guard.read_only_inspections(), 4);
    // Not inspection: a build, a test, a bare echo, a web tool.
    for (tool, input) in [
        ("Bash", bash_input("cargo test -p some-crate guard")),
        ("Bash", bash_input("cargo check -p some-crate")),
        ("Bash", bash_input("echo done")),
        ("WebFetch", json!({"url": "https://example.invalid"})),
    ] {
        assert_eq!(guard.before_tool(tool, &input), None, "{tool} {input}");
        assert_eq!(guard.result_note(tool, &input), None, "{tool} {input}");
    }
    assert_eq!(guard.read_only_inspections(), 4);
    // Shell inspection is the fifth and is refused.
    let refusal = guard
        .before_tool("Bash", &bash_input("sed -n '1,40p' src/lib.rs"))
        .unwrap();
    assert!(refusal.starts_with(READ_CEILING_MARKER), "{refusal}");
}

#[test]
fn the_soft_ceiling_appends_the_nudge_from_the_threshold_on() {
    let guard = read_only(3, 6);
    for call in 1..=2 {
        let (verdict, note) = inspect(&guard, "Read", &read_input());
        assert_eq!(verdict, None, "call {call}");
        assert_eq!(note, None, "call {call} is under the soft ceiling");
    }
    let (verdict, note) = inspect(&guard, "Grep", &json!({"pattern": "x"}));
    assert_eq!(verdict, None);
    assert_eq!(
        note.as_deref(),
        Some(
            "You have made 3 inspection calls; produce your deliverable now — further reading past 6 will be refused."
        )
    );
    let (verdict, note) = inspect(&guard, "Bash", &bash_input("git diff --stat"));
    assert_eq!(verdict, None, "the soft ceiling never refuses");
    assert_eq!(
        note.as_deref(),
        Some(
            "You have made 4 inspection calls; produce your deliverable now — further reading past 6 will be refused."
        )
    );
    // A build between two reads is neither counted nor annotated.
    let (verdict, note) = inspect(&guard, "Bash", &bash_input("cargo test -p some-crate"));
    assert_eq!((verdict, note), (None, None));
    assert_eq!(guard.read_only_inspections(), 4);
}

#[test]
fn the_hard_ceiling_refuses_at_the_threshold_and_keeps_refusing() {
    let guard = read_only(2, 4);
    for _ in 0..4 {
        assert_eq!(guard.before_tool("Read", &read_input()), None);
    }
    let expected = format!(
        "{READ_CEILING_MARKER} 4 inspection calls; answer now with your deliverable from what you have read. Further Read, Grep, Glob and shell inspection calls are refused; build and test commands still run."
    );
    for tool_input in [
        ("Read", read_input()),
        ("Grep", json!({"pattern": "x"})),
        ("Glob", json!({"pattern": "*.rs"})),
        ("read-own-evidence", json!({"offset": 0})),
        ("Bash", bash_input("cat src/lib.rs")),
        ("Bash", bash_input("rg -n needle src")),
        ("Read", read_input()),
    ] {
        let (tool, input) = tool_input;
        assert_eq!(
            guard.before_tool(tool, &input).as_deref(),
            Some(expected.as_str()),
            "{tool}"
        );
    }
    assert_eq!(
        guard.read_only_inspections(),
        4,
        "refused calls are not counted"
    );
    // The session is not ended: no terminal failure, and build/test still run.
    assert_eq!(guard.terminal_failure(), None);
    for command in [
        "cargo test -p some-crate guard",
        "cargo build --bin app",
        "npm test",
        "pytest -q",
    ] {
        assert_eq!(
            guard.before_tool("Bash", &bash_input(command)),
            None,
            "{command}"
        );
    }
    assert_eq!(guard.terminal_failure(), None);
}

#[test]
fn a_zero_ceiling_is_off() {
    let nudge_only = read_only(2, 0);
    for _ in 0..300 {
        assert_eq!(nudge_only.before_tool("Read", &read_input()), None);
    }
    assert_eq!(
        nudge_only.result_note("Read", &read_input()).as_deref(),
        Some("You have made 300 inspection calls; produce your deliverable now.")
    );
    let refuse_only = read_only(0, 2);
    for _ in 0..2 {
        let (verdict, note) = inspect(&refuse_only, "Read", &read_input());
        assert_eq!((verdict, note), (None, None));
    }
    assert!(refuse_only.before_tool("Read", &read_input()).is_some());
    let off = read_only(0, 0);
    for _ in 0..300 {
        let (verdict, note) = inspect(&off, "Glob", &json!({"pattern": "*"}));
        assert_eq!((verdict, note), (None, None));
    }
    assert_eq!(off.preamble(), None);
}

#[test]
fn the_defaults_are_eighty_and_one_hundred_twenty() {
    let settings = WorkflowReadGuardSettings::default();
    assert_eq!(settings.read_only_soft_call_ceiling, 80);
    assert_eq!(settings.read_only_hard_call_ceiling, 120);
    let guard = WorkflowReadGuard::shell_only(&settings);
    for call in 1..=120 {
        let (verdict, note) = inspect(&guard, "Read", &read_input());
        assert_eq!(verdict, None, "call {call}");
        assert_eq!(note.is_some(), call >= 80, "call {call}: {note:?}");
    }
    let refusal = guard.before_tool("Read", &read_input()).unwrap();
    assert!(
        refusal.starts_with("read ceiling reached: 120 inspection calls;"),
        "{refusal}"
    );
}

#[test]
fn the_preamble_states_both_ceilings_for_a_read_only_call_only() {
    let both = read_only(80, 120).preamble().unwrap();
    assert!(
        both.starts_with("Inspection ceiling for this read-only call:"),
        "{both}"
    );
    assert!(both.contains("from 80 inspection calls"), "{both}");
    assert!(
        both.contains("past 120 such calls further reading is refused"),
        "{both}"
    );
    assert!(
        both.contains("build and test commands are not counted"),
        "{both}"
    );
    assert_eq!(both.matches(". ").count(), 0, "one sentence: {both}");
    let soft_only = read_only(80, 0).preamble().unwrap();
    assert!(
        soft_only.contains("from 80 inspection calls") && !soft_only.contains("refused"),
        "{soft_only}"
    );
    let hard_only = read_only(0, 120).preamble().unwrap();
    assert!(
        hard_only.contains("past 120 inspection calls") && !hard_only.contains("reminds"),
        "{hard_only}"
    );
    let writer = WorkflowReadGuard::from_settings(&settings(80, 120));
    assert_eq!(writer.mode(), GuardMode::WriteCapable);
    assert_eq!(writer.preamble(), None);
}

#[test]
fn the_write_capable_guard_never_counts_notes_or_refuses_on_the_ceilings() {
    // Ceilings tighter than the budget: if they applied, the first read would
    // be refused and every read annotated. The budget alone must speak.
    let guard = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings {
        max_reads_before_first_write: 5,
        read_only_soft_call_ceiling: 1,
        read_only_hard_call_ceiling: 2,
        ..Default::default()
    });
    for _ in 0..5 {
        let (verdict, note) = inspect(&guard, "Read", &read_input());
        assert_eq!((verdict, note), (None, None));
    }
    assert_eq!(guard.read_only_inspections(), 0);
    let refusal = guard.before_tool("Read", &read_input()).unwrap();
    assert!(
        refusal.starts_with("read budget exhausted (5 reads, 0 substantive writes)."),
        "{refusal}"
    );
    assert!(!refusal.contains(READ_CEILING_MARKER));
    assert_eq!(guard.result_note("Read", &read_input()), None);
}

#[tokio::test]
async fn a_ceiling_refusal_is_recorded_like_any_other() {
    let temp = tempfile::tempdir().unwrap();
    let sidecar = temp.path().join("read-set.jsonl");
    let guard = super::scope_read_set(sidecar.clone(), async { read_only(0, 1) }).await;
    assert_eq!(guard.before_tool("Read", &read_input()), None);
    assert!(!sidecar.exists(), "an admitted read leaves no record");
    assert!(
        guard
            .before_tool("Grep", &json!({"pattern": "needle", "path": "src"}))
            .is_some()
    );
    let records: Vec<Value> = std::fs::read_to_string(&sidecar)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(records[0]["kind"], REFUSAL_RECORD_KIND);
    assert_eq!(records[0]["tool"], "Grep");
    assert!(
        records[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with(READ_CEILING_MARKER)
    );
    assert_eq!(records[1]["kind"], TOOL_CALL_RECORD_KIND);
    assert!(
        records[1]["status"]
            .as_str()
            .unwrap()
            .starts_with("refused: read ceiling reached:")
    );
}
