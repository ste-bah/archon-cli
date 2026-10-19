//! Batch O: an empty write set and prose forbidden entries are findings.

use std::path::{Path, PathBuf};

use super::*;

fn task(owns: &[&str], forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: "TASK-X-001".into(),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

fn raw(owns: &str, forbids: &str) -> String {
    format!("## Files Expected to Change\n\n{owns}\n\n## Files Forbidden to Change\n\n{forbids}\n")
}

fn grounded_task_set() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repository = temp.path().join("repository");
    let tasks = project.join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir_all(repository.join("scripts")).unwrap();
    let repository_root = repository.canonicalize().unwrap();
    let declared = format!("{}/scripts/x.sh", repository_root.display());
    let body = format!(
        "```yaml\ntask_id: TASK-X-001\ntitle: Example\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n\n- `{declared}` — exists\n\n## Files Forbidden to Change\n\n- scripts/x.sh\n\n## Focused Tests\n\n- `cargo test -p example`\n"
    );
    let record = archon_workflow::repository_record::RepositoryRecordV1 {
        schema_version: archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
        repository_root: repository_root.display().to_string(),
        base_commit: archon_workflow::repository_record::UNBORN_BASE_COMMIT.into(),
        decomposition_run_id: "test".into(),
        recorded_at: "now".into(),
    };
    archon_workflow::repository_record::write_repository_record(&tasks, &record).unwrap();
    std::fs::write(tasks.join("TASK-X-001.md"), &body).unwrap();
    (temp, project, tasks, body)
}

#[test]
fn body_and_set_gates_use_the_recorded_repository_root_for_absolute_declarations() {
    let (_temp, project, tasks, body) = grounded_task_set();
    let task_path = tasks.join("TASK-X-001.md");
    let body_gate = super::super::task_file::inspect_raw(
        &project,
        &task_path,
        &body,
        archon_core::config::GateMode::Enforce,
    );
    let expected = body_gate
        .blockers
        .iter()
        .find(|finding| finding.contains("declared `scripts/x.sh`"))
        .expect("body gate reports the declared and forbidden path collision");
    let set_gate = set_findings(&tasks);
    let set_collision = set_gate
        .iter()
        .map(|finding| &finding.text)
        .find(|finding| finding.contains("declared `scripts/x.sh`"))
        .expect("set gate reports the declared and forbidden path collision");
    assert_eq!(expected, set_collision);
}

#[test]
fn missing_repository_record_keeps_each_gate_fallback_root() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let tasks = project.join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let repository = temp.path().join("repository");
    std::fs::create_dir_all(repository.join("scripts")).unwrap();
    let body = format!(
        "```yaml\ntask_id: TASK-X-001\ntitle: Example\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n\n- `{}/scripts/x.sh` — exists\n\n## Files Forbidden to Change\n\n- scripts/x.sh\n\n## Focused Tests\n\n- `cargo test -p example`\n",
        repository.display()
    );
    let task_path = tasks.join("TASK-X-001.md");
    std::fs::write(&task_path, &body).unwrap();
    let body_gate = super::super::task_file::inspect_raw(
        &project,
        &task_path,
        &body,
        archon_core::config::GateMode::Enforce,
    );
    assert!(
        body_gate
            .blockers
            .iter()
            .all(|finding| !finding.contains("declared `scripts/x.sh`")),
        "{:#?}",
        body_gate.blockers
    );
    assert!(
        set_findings(&tasks)
            .iter()
            .all(|finding| !finding.text.contains("declared `scripts/x.sh`"))
    );
}

#[test]
fn a_task_that_declares_no_file_is_a_finding_naming_it() {
    let findings = inspect(&task(&[], &[]), "", Path::new("/repo"));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].starts_with("task TASK-X-001:")
            && findings[0].contains("Files Expected to Change"),
        "{findings:?}"
    );
    let mut shared = task(&[], &[]);
    shared.shared_append_target_files = vec!["`src/registry.rs`".into()];
    assert!(inspect(&shared, "", Path::new("/repo")).is_empty());
}

#[test]
fn catch_all_forbidden_prose_is_a_finding_and_literal_entries_are_not() {
    let task = task(
        &["`src/a.rs` — exists (3 lines)"],
        &[
            "Everything else in the repository",
            "`docs/`",
            "crates/x/src/store.rs — owned by another task",
            ".mcp.json",
            "*.lock",
        ],
    );
    let findings = inspect(
        &task,
        &raw(
            "- `src/a.rs` — exists (3 lines)",
            "- Everything else in the repository\n- `docs/`\n- crates/x/src/store.rs — owned by another task\n- .mcp.json\n- *.lock",
        ),
        Path::new("/repo"),
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].contains("Everything else in the repository"),
        "{findings:?}"
    );
}

#[test]
fn declared_path_forbidden_by_qualified_or_bare_bullet_is_a_scope_finding() {
    for forbidden in [
        "- `src/owned.rs` existing test functions `checks_owned`",
        "- `src/owned.rs`",
    ] {
        let task = task(
            &["`src/owned.rs` — exists (3 lines)"],
            &[forbidden.trim_start_matches("- ")],
        );
        let findings = inspect(
            &task,
            &raw("- `src/owned.rs` — exists (3 lines)", forbidden),
            Path::new("/repo"),
        );
        let collision = findings
            .iter()
            .find(|finding| finding.contains("declared `src/owned.rs`"))
            .unwrap_or_else(|| panic!("{findings:?}"));
        assert!(
            collision.contains("is also named by forbidden bullet"),
            "{collision}"
        );
        assert!(
            collision.contains(forbidden.trim_start_matches("- ")),
            "{collision}"
        );
        assert!(
            collision.contains("forbidden bullets naming files forbid whole files"),
            "{collision}"
        );
    }
}

#[test]
fn declared_path_without_forbidden_collision_has_no_scope_finding() {
    let task = task(&["`src/owned.rs` — exists (3 lines)"], &["`src/other.rs`"]);
    assert!(
        inspect(
            &task,
            &raw("- `src/owned.rs` — exists (3 lines)", "- `src/other.rs"),
            Path::new("/repo")
        )
        .is_empty()
    );
}

#[test]
fn the_two_review_blockers_agree_on_scope_collisions() {
    let false_collision = "## Files Expected to Change\n\n- `src/a.rs` — updates the `state.load` helper\n\n## Files Forbidden to Change\n\n- `**/state.load`\n";
    let first_task = task(&["src/a.rs"], &["**/state.load"]);
    assert!(inspect(&first_task, false_collision, Path::new("/repo")).is_empty());
    let binding = archon_knowledge::traceability::tasks::parse_task_binding(
        &format!("```yaml\ntask_id: TASK-X-001\nimplements: []\n```\n\n{false_collision}"),
        "TASK-X-001.md",
    )
    .unwrap();
    assert!(
        archon_write_plan::declaration_conflicts(
            binding.path_scopes.iter().filter_map(|path| {
                archon_write_plan::normalize_declared_path(path, Path::new("/repo"))
            }),
            &first_task.files_forbidden_to_change
        )
        .is_empty()
    );

    let real_collision = "## Files Expected to Change\n\n- src/a.rs — exists\n\n## Files Forbidden to Change\n\n- src/a.rs\n";
    let second_task = task(&["src/a.rs"], &["src/a.rs"]);
    let body = inspect(&second_task, real_collision, Path::new("/repo"));
    let body_conflict = body
        .iter()
        .find(|finding| finding.contains("declared `src/a.rs`"))
        .expect("body gate must refuse the plain path");
    let binding = archon_knowledge::traceability::tasks::parse_task_binding(
        &format!("```yaml\ntask_id: TASK-X-001\nimplements: []\n```\n\n{real_collision}"),
        "TASK-X-001.md",
    )
    .unwrap();
    let trace_conflict = archon_write_plan::declaration_conflicts(
        binding.path_scopes.iter().filter_map(|path| {
            archon_write_plan::normalize_declared_path(path, Path::new("/repo"))
        }),
        &second_task.files_forbidden_to_change,
    );
    assert_eq!(trace_conflict.len(), 1);
    assert_eq!(
        body_conflict,
        &format!("task TASK-X-001: {}", trace_conflict[0].finding())
    );
}

#[test]
fn body_and_trace_collision_sets_match_for_six_bullet_and_forbidden_shapes() {
    let cases = [
        ("`src/a.rs` — exists", "src/a.rs"),
        ("src/a.rs — exists", "src/a.rs"),
        (
            "`src/a.rs` updates `state.load` and `helper.rs`",
            "src/a.rs",
        ),
        ("`./src/a.rs` exists", "src/a.rs"),
        ("Likely anchors: `src/a.rs`, `src/b.rs`", "src/a.rs"),
        ("src/a.rs — exists", "src/"),
        ("src/a.rs — exists", "src/*.rs"),
    ];
    for (bullet, forbidden) in cases {
        let body = format!(
            "## Files Expected to Change\n\n- {bullet}\n\n## Files Forbidden to Change\n\n- {forbidden}\n"
        );
        let parsed = task(&["src/a.rs"], &[forbidden]);
        let body_findings = inspect(&parsed, &body, Path::new("/repo"));
        let body_paths = body_findings
            .iter()
            .filter_map(|finding| {
                finding
                    .split("declared `")
                    .nth(1)
                    .and_then(|tail| tail.split('`').next())
                    .map(str::to_string)
            })
            .collect::<std::collections::BTreeSet<_>>();
        let binding = archon_knowledge::traceability::tasks::parse_task_binding(
            &format!("```yaml\ntask_id: TASK-X-001\nimplements: []\n```\n\n{body}"),
            "TASK-X-001.md",
        )
        .unwrap();
        let trace_paths = archon_write_plan::declaration_conflicts(
            binding.path_scopes.iter().filter_map(|path| {
                archon_write_plan::normalize_declared_path(path, Path::new("/repo"))
            }),
            &parsed.files_forbidden_to_change,
        )
        .into_iter()
        .map(|conflict| conflict.path)
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            body_paths, trace_paths,
            "bullet={bullet:?}, forbidden={forbidden:?}"
        );
    }
}

#[test]
fn review_examples_use_the_opening_subject_and_shared_path_normalization() {
    let root = Path::new("/repo");
    let prose = "## Files Expected to Change\n\n- `state_load` updates `src/a.rs`\n\n## Files Forbidden to Change\n\n- src/a.rs\n";
    let findings = inspect(&task(&[], &["src/a.rs"]), prose, root);
    assert!(
        findings
            .iter()
            .all(|finding| !finding.contains("declared `src/a.rs`")),
        "{findings:?}"
    );

    let labeled = "## Files Expected to Change\n\n- Likely anchors: `src/a.rs`, `src/b.rs`\n\n## Files Forbidden to Change\n\n- src/a.rs\n";
    let findings = inspect(&task(&[], &["src/a.rs"]), labeled, root);
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("declared `src/a.rs`")),
        "{findings:?}"
    );

    for (declared, forbidden, expected) in [
        ("/repo/src/a.rs — exists", "src/a.rs", "src/a.rs"),
        ("`.mcp.json` — exists", ".mcp.json", ".mcp.json"),
    ] {
        let body = format!(
            "## Files Expected to Change\n\n- {declared}\n\n## Files Forbidden to Change\n\n- {forbidden}\n"
        );
        let findings = inspect(&task(&[], &[forbidden]), &body, root);
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains(&format!("declared `{expected}`"))),
            "{findings:?}"
        );
    }
}
