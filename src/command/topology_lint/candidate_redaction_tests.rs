//! Issue 248: the body gate refuses a candidate that carries the
//! log-redaction marker as a standalone word, naming where it is.

use archon_core::config::GateMode;
use archon_workflow::events::REDACTION_MARKER;

fn candidate(focused_test: &str) -> String {
    format!(
        "# TASK-WS-001\n\n```yaml\ntask_id: TASK-WS-001\ntitle: T\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"AC-WS-001\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n\n- crates/w/src/lib.rs\n\n## Focused Tests\n\n- `{focused_test}`\n"
    )
}

fn findings(raw: &str) -> Vec<String> {
    let temp = tempfile::tempdir().expect("tempdir");
    let cwd = temp.path();
    let tasks = cwd.join("tasks").join("PRD-WS-001");
    std::fs::create_dir_all(&tasks).expect("tasks");
    std::fs::write(
        cwd.join("tasks").join("PRD-WS-001.md"),
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-WS-001 | Widgets land. |\n",
    )
    .expect("prd");
    let path = tasks.join("TASK-WS-001.md");
    super::evaluate_task_file_candidate(cwd, &path, raw.as_bytes(), GateMode::Enforce)
        .expect("mechanical checks")
        .findings
        .into_iter()
        .map(|finding| finding.text)
        .collect()
}

#[test]
fn a_task_body_holding_the_redaction_marker_is_refused_with_its_line() {
    let marked = candidate(&format!(
        "cargo test -p w -- --token {REDACTION_MARKER} --nocapture"
    ));
    let line = marked
        .lines()
        .position(|line| line.contains(REDACTION_MARKER))
        .expect("marked line")
        + 1;
    let texts = findings(&marked);
    assert!(
        texts
            .iter()
            .any(|text| text.contains("log-redaction marker")
                && text.contains(REDACTION_MARKER)
                && text.contains(&format!("line {line}"))),
        "{texts:?}"
    );
}

#[test]
fn a_task_body_quoting_the_marker_inside_a_word_is_not_refused() {
    let quoted = candidate(&format!("grep -q '{REDACTION_MARKER}' crates/w/src/lib.rs"));
    let texts = findings(&quoted);
    assert!(
        texts
            .iter()
            .all(|text| !text.contains("log-redaction marker")),
        "{texts:?}"
    );
}
