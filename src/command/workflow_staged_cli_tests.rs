use super::*;

/// The frozen file the fixtures below are bound to.
fn ws() -> &'static Path {
    Path::new("tasks/TASK-WS-001.md")
}

/// Issue-61: the bytes the gate lints and stages are the document inside
/// an author's outer fence; a task file that opens with its frontmatter
/// passes through byte-for-byte, as does a candidate that is not UTF-8.
#[test]
fn the_body_gate_unwraps_an_outer_fence_before_lint_and_staging() {
    let wrapped = b"```markdown\n```yaml\ntask_id: TASK-WS-001\n```\n\n## Focused Tests\n\n- `cargo test -p w`\n```\n".to_vec();
    let landed = candidate::normalize_task_candidate(wrapped, ws()).unwrap();
    assert!(landed.unwrapped());
    assert_eq!(
        landed.bytes,
        b"```yaml\ntask_id: TASK-WS-001\n```\n\n## Focused Tests\n\n- `cargo test -p w`\n".to_vec()
    );
    let plain = b"```yaml\ntask_id: TASK-WS-001\n```\n\n# TASK-WS-001\n".to_vec();
    let landed = candidate::normalize_task_candidate(plain.clone(), ws()).unwrap();
    assert_eq!((landed.bytes.clone(), landed.unwrapped()), (plain, false));
    // A heading before the frontmatter is packaging the host discards,
    // recorded in the report; the task file after it lands unchanged.
    let headed = b"# TASK-WS-001\n\n```yaml\ntask_id: TASK-WS-001\n```\n".to_vec();
    let landed = candidate::normalize_task_candidate(headed, ws()).unwrap();
    assert_eq!(
        landed.bytes,
        b"```yaml\ntask_id: TASK-WS-001\n```\n".to_vec()
    );
    assert_eq!(landed.packaging.map(|p| p.lines), Some(2));
    // No frontmatter with a task_id at all is still refused.
    let headed = b"# TASK-WS-001\n\nno task file\n".to_vec();
    assert!(candidate::normalize_task_candidate(headed, ws()).is_err());
    let not_utf8 = vec![0x60, 0x60, 0x60, 0x0a, 0xff, 0xfe];
    let landed = candidate::normalize_task_candidate(not_utf8.clone(), ws()).unwrap();
    assert_eq!(
        (landed.bytes.clone(), landed.unwrapped()),
        (not_utf8, false)
    );
}

/// Issue-44: the body gate must refuse, not pass, when the critic cannot
/// be reached — the same rule as the set gate. A mechanically clean body
/// with no provider configured leaves the envelope with an operational
/// error naming the fidelity audit and no invented finding.
#[tokio::test]
async fn the_body_gate_reports_an_unreachable_critic_as_operational_never_a_pass() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cwd = temp.path();
    let tasks = cwd.join("tasks").join("PRD-WS-001");
    std::fs::create_dir_all(&tasks).expect("tasks");
    std::fs::write(
        cwd.join("tasks").join("PRD-WS-001.md"),
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-WS-001 | Widgets land. |\n",
    )
    .expect("prd");
    let candidate = "```yaml\ntask_id: TASK-WS-001\ntitle: T\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"AC-WS-001\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n# TASK-WS-001\n\n## Files Expected to Change\n\n- crates/w/src/lib.rs\n\n## Focused Tests\n\n- `cargo test -p w`\n";
    let path = tasks.join("TASK-WS-001.md");
    let clean = crate::command::topology_lint::evaluate_task_file_candidate(
        cwd,
        &path,
        candidate.as_bytes(),
        archon_core::config::GateMode::Enforce,
    )
    .expect("mechanical checks");
    assert!(clean.findings.is_empty(), "{}", clean.report);
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = archon_core::config::GateMode::Enforce;
    let env = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
    let evaluation = audit_candidate_fidelity(
        cwd,
        &path,
        candidate.as_bytes(),
        "call-1",
        &config,
        &env,
        clean,
    )
    .await;
    let error = evaluation
        .operational_error()
        .expect("operational error recorded");
    assert!(
        error.contains("obligation fidelity audit failed operationally"),
        "{error}"
    );
    assert!(error.contains("critic client"), "{error}");
    assert!(
        evaluation.findings.is_empty(),
        "no finding stands in for a verdict"
    );
    assert!(!path.exists(), "the gate publishes nothing on its own");
}

/// The freeze point must refuse, not pass, when the critic cannot be
/// reached: an unconfigured provider yields an envelope whose
/// `operational_error` names the fidelity audit, and no policy finding is
/// invented to stand in for the verdict that was never given.
/// Issue 259 review: an attempt the host kills before it saves its first
/// call batch (building the client, say) still leaves a progress line,
/// so the next attempt's saved work reads as growth, not as an attempt
/// with no evidence.
#[tokio::test]
async fn the_set_gate_reports_a_progress_baseline_before_it_builds_anything() {
    if std::env::var_os("ARCHON_TEST_BLOCK_CLIENT_BUILD").is_none() {
        super::super::workflow_freeze_cli::baseline_tests::assert_entry_baseline(concat!(
            module_path!(),
            "::the_set_gate_reports_a_progress_baseline_before_it_builds_anything"
        ));
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let config = archon_core::config::ArchonConfig::default();
    let env = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
    handle_staged_task_set_lint(
        temp.path(),
        None,
        Some(&temp.path().join("tasks")),
        None,
        None,
        Some(&temp.path().join("staging/envelope.json")),
        Some("baseline-test"),
        &config,
        &env,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_set_gate_reports_an_unreachable_critic_as_operational_never_a_pass() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cwd = temp.path();
    let tasks = cwd.join("tasks").join("PRD-WS-001");
    std::fs::create_dir_all(&tasks).expect("tasks");
    std::fs::write(
        cwd.join("tasks").join("PRD-WS-001.md"),
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-WS-001 | Widgets land. |\n",
    )
    .expect("prd");
    std::fs::write(
            tasks.join("TASK-WS-001.md"),
            "# TASK-WS-001\n\n```yaml\ntask_id: TASK-WS-001\ntitle: T\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"AC-WS-001\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Focused Tests\n\n- `cargo test -p w`\n",
        )
        .expect("task");
    let staging = cwd.join("staging");
    std::fs::create_dir_all(&staging).expect("staging");
    let envelope = staging.join("gate-envelope.json");
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = archon_core::config::GateMode::Enforce;
    let env = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
    handle_staged_task_set_lint(
        cwd,
        None,
        Some(&tasks),
        None,
        None,
        Some(&envelope),
        Some("call-1"),
        &config,
        &env,
    )
    .await
    .expect("the staged child reports through the envelope, never by exiting non-zero");
    let text = std::fs::read_to_string(&envelope).expect("envelope written");
    let value: serde_json::Value = serde_json::from_str(&text).expect("envelope json");
    let error = value["operational_error"]["text"]
        .as_str()
        .expect("operational error recorded");
    assert!(
        error.contains("obligation fidelity audit failed operationally"),
        "{error}"
    );
    assert!(error.contains("critic client"), "{error}");
}
