use super::*;

fn write_prd(dir: &Path, bullets: &[&str]) -> PathBuf {
    let path = dir.join("PRD.md");
    let body: String = bullets.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(&path, body).expect("write PRD");
    path
}

#[test]
fn trace_calls_the_shared_obligation_extractor() {
    let source = include_str!("../requirement_trace.rs");
    assert!(
        source.contains("archon_workflow::obligation_ids::obligation_ids(&prd)"),
        "trace coverage must call the same extractor as freeze and topology lint"
    );
}

#[test]
fn table_obligations_share_coverage_without_inventing_anchor_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = dir.path().join("PRD.md");
    std::fs::write(
        &prd,
        "- REQ-X-001: rich requirement text\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | accepted when checked |\n",
    )
    .expect("write PRD");
    let tasks = dir.path().join("tasks");
    std::fs::create_dir(&tasks).expect("tasks");
    std::fs::write(
        tasks.join("TASK-X-001.md"),
        "```yaml\ntask_id: TASK-X-001\nimplements: [REQ-X-001, AC-X-001]\n```\n",
    )
    .expect("write task");

    let report = build_report(dir.path(), &TraceOptions::new(prd, tasks)).expect("report");
    assert_eq!(report.coverage.requirements_total, 2);
    assert!(
        report.coverage.phantom.is_empty(),
        "{:?}",
        report.coverage.phantom
    );
    assert!(
        report.coverage.unclaimed.is_empty(),
        "{:?}",
        report.coverage.unclaimed
    );
    assert_eq!(
        report.rows.len(),
        1,
        "only REQ bullets produce rich proof rows"
    );
    assert_eq!(report.rows[0].requirement_id, "REQ-X-001");
}

fn write_trace_task(dir: &Path, body: &str) -> PathBuf {
    let tasks = dir.join("tasks");
    std::fs::create_dir_all(&tasks).expect("tasks");
    std::fs::write(tasks.join("TASK-X-001.md"), body).expect("write task");
    tasks
}

#[test]
fn cleanly_parsed_coverage_gap_keeps_the_normal_report_before_failing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(dir.path(), &["- REQ-X-001: orphaned."]);
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\nimplements: []\n```\n",
    );
    let verdict = run_trace(dir.path(), &TraceOptions::new(prd, tasks)).expect("verdict");
    assert!(
        verdict.report.contains("Per requirement"),
        "{}",
        verdict.report
    );
    assert!(verdict.report.contains("REQ-X-001"), "{}", verdict.report);
    assert!(
        verdict
            .report
            .contains("BLOCKING trace input/coverage findings")
    );
    assert!(verdict.require_clean().is_err());
}

#[test]
fn trace_verdict_blocks_only_deterministic_coverage_defects_with_remedies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(
        dir.path(),
        &["- REQ-X-001: claimed.", "- REQ-X-002: orphan."],
    );
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\nimplements: [REQ-X-001, REQ-X-404]\n```\n",
    );

    let verdict = run_trace(dir.path(), &TraceOptions::new(prd, tasks)).expect("verdict");
    assert_eq!(verdict.blocking_findings.len(), 2, "{verdict:?}");
    assert!(
        verdict.blocking_findings.iter().any(|finding| {
            finding.contains("TASK-X-001")
                && finding.contains("REQ-X-404")
                && finding.contains("remove it from")
                && finding.contains("or correct it")
        }),
        "{:?}",
        verdict.blocking_findings
    );
    assert!(
        verdict.blocking_findings.iter().any(|finding| {
            finding.contains("REQ-X-002")
                && finding.contains("add it to at least one TASK file's implements list")
        }),
        "{:?}",
        verdict.blocking_findings
    );
    assert!(
        !verdict
            .blocking_findings
            .iter()
            .any(|finding| finding.contains("code index")),
        "missing optional proof evidence is not an input/coverage blocker: {:?}",
        verdict.blocking_findings
    );
}

#[test]
fn trace_cli_and_slash_call_the_typed_gate_after_rendering() {
    let cli = include_str!("../requirement_trace.rs");
    assert!(
        cli.contains("workflow_gate::run_sync_gate")
            && cli.contains("config.workflow.gate_mode")
            && cli.contains("evaluate_trace")
            && cli.contains("write_cli_report(&mut stdout.lock(), disposition.report())?")
            && cli.contains("disposition.require_allowed()"),
        "CLI must evaluate through the startup-mode disposition, render, then block"
    );
    let slash = include_str!("slash.rs");
    assert!(
        slash.contains("workflow_gate::run_sync_gate")
            && slash.contains("ctx.gate_mode")
            && slash.contains("evaluate_trace")
            && slash.contains("TuiEvent::TextDelta(disposition.report()")
            && slash.contains("disposition.require_allowed()"),
        "slash trace must use the same startup-mode disposition after rendering"
    );
}

#[test]
fn malformed_task_binding_names_the_exact_yaml_edit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(dir.path(), &["- REQ-X-001: claimed."]);
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\nimplements:\n  - REQ-X-001\n```\n",
    );
    let verdict = run_trace(dir.path(), &TraceOptions::new(prd, tasks))
        .expect("malformed input must produce a rendered typed verdict");
    assert!(
        verdict.report.contains("BLOCKING trace input"),
        "{}",
        verdict.report
    );
    assert_eq!(verdict.blocking_findings.len(), 1, "{verdict:?}");
    let finding = &verdict.blocking_findings[0];
    assert!(finding.contains("TASK-X-001.md"), "{finding}");
    assert!(finding.contains("implements: [REQ-DL-020]"), "{finding}");
    assert!(verdict.require_clean().is_err());
}

#[test]
fn live_corpus_has_104_shared_obligations_and_no_coverage_blockers() {
    let (Ok(prd), Ok(tasks)) = (
        std::env::var("ARCHON_TRACE_PRD"),
        std::env::var("ARCHON_TRACE_TASKS"),
    ) else {
        eprintln!("SKIPPED: set ARCHON_TRACE_PRD and ARCHON_TRACE_TASKS to run live measurement");
        return;
    };
    let tasks = PathBuf::from(tasks);
    let parsed_files = std::fs::read_dir(&tasks)
        .expect("read live task corpus")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("TASK-") && name.ends_with(".md"))
        })
        .count();
    let verdict = run_trace(
        Path::new("/"),
        &TraceOptions::new(PathBuf::from(prd), tasks),
    )
    .expect("live corpus trace");
    let report = build_report(
        Path::new("/"),
        &TraceOptions::new(
            PathBuf::from(std::env::var("ARCHON_TRACE_PRD").unwrap()),
            PathBuf::from(std::env::var("ARCHON_TRACE_TASKS").unwrap()),
        ),
    )
    .expect("live corpus report");

    assert_eq!(parsed_files, 18);
    assert_eq!(report.coverage.requirements_total, 104);
    assert!(
        report.coverage.phantom.is_empty(),
        "{:?}",
        report.coverage.phantom
    );
    assert!(
        report.coverage.unclaimed.is_empty(),
        "{:?}",
        report.coverage.unclaimed
    );
    assert!(
        verdict.blocking_findings.is_empty(),
        "{:?}",
        verdict.blocking_findings
    );
}

#[test]
fn empty_prd_and_task_populations_are_not_a_synthetic_pass() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = dir.path().join("PRD.md");
    std::fs::write(&prd, "# Empty PRD\n").expect("write PRD");
    let tasks = dir.path().join("tasks");
    std::fs::create_dir(&tasks).expect("tasks");

    let verdict = run_trace(dir.path(), &TraceOptions::new(prd, tasks)).expect("typed verdict");
    assert!(
        verdict.blocking_findings.iter().any(|finding| {
            finding.contains("defines zero obligations")
                && finding.contains("add a line-leading REQ-<AREA>-<NNN> bullet")
        }),
        "{:?}",
        verdict.blocking_findings
    );
    assert!(
        verdict.blocking_findings.iter().any(|finding| {
            finding.contains("contains zero TASK-*.md files")
                && finding.contains("add the decomposed TASK files or correct --tasks")
        }),
        "{:?}",
        verdict.blocking_findings
    );
    assert!(verdict.require_clean().is_err());
}

#[test]
fn malformed_binding_does_not_invent_secondary_coverage_findings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(dir.path(), &["- REQ-X-001: claimed by malformed input."]);
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\nimplements:\n  - REQ-X-001\n```\n",
    );
    let verdict = run_trace(dir.path(), &TraceOptions::new(prd, tasks)).expect("typed verdict");
    assert_eq!(verdict.blocking_findings.len(), 1, "{verdict:?}");
    assert!(verdict.blocking_findings[0].contains("implements: [REQ-DL-020]"));
    assert!(
        !verdict.report.contains("claimed by no task"),
        "{}",
        verdict.report
    );
}

#[test]
fn trace_off_skips_inputs_and_observe_keeps_json_report_machine_valid() {
    let off = tempfile::tempdir().unwrap();
    let options = TraceOptions::new(
        off.path().join("missing-prd.md"),
        off.path().join("missing-tasks"),
    );
    let disposition = crate::command::workflow_gate::run_sync_gate(
        off.path(),
        archon_core::config::GateMode::Off,
        crate::command::workflow_gate::GateId::RequirementsTrace,
        || evaluate_trace(off.path(), &options),
    )
    .unwrap();
    assert_eq!(
        disposition.report(),
        crate::command::workflow_gate::OFF_MESSAGE
    );
    assert!(!off.path().join(".archon").exists());

    let observed = tempfile::tempdir().unwrap();
    let prd = write_prd(observed.path(), &["- REQ-X-001: unclaimed."]);
    let tasks = observed.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join("TASK-X-010.md"),
        "```yaml\ntask_id: TASK-X-010\ntitle: X\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n",
    )
    .unwrap();
    let mut options = TraceOptions::new(prd, tasks);
    options.json = true;
    let disposition = crate::command::workflow_gate::run_sync_gate(
        observed.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::RequirementsTrace,
        || evaluate_trace(observed.path(), &options),
    )
    .unwrap();
    serde_json::from_str::<serde_json::Value>(disposition.report())
        .expect("observe stdout report remains JSON");
    assert!(
        disposition
            .diagnostics()
            .iter()
            .any(|line| line.contains("REQ-X-001"))
    );
}

#[test]
fn malformed_trace_population_preserves_report_but_is_operational_in_observe() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(dir.path(), &["- REQ-X-001: claimed."]);
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\nimplements:\n  - REQ-X-001\n```\n",
    );
    let options = TraceOptions::new(prd, tasks);
    let disposition = crate::command::workflow_gate::run_sync_gate(
        dir.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::RequirementsTrace,
        || evaluate_trace(dir.path(), &options),
    )
    .unwrap();

    assert!(disposition.report().contains("Coverage: NOT COMPUTED"));
    assert!(disposition.report().contains("TASK-X-001.md"));
    let error = disposition.require_allowed().unwrap_err().to_string();
    assert!(error.contains("traceability input error"), "{error}");
    assert!(!crate::command::workflow_gate::shadow_log_path(dir.path()).exists());
}

#[test]
fn malformed_prd_obligation_id_is_a_typed_trace_finding() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(
        dir.path(),
        &["- REQ-X-001: valid.", "- REQ-X2-002: malformed area."],
    );
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\nimplements: [REQ-X-001]\n```\n",
    );
    let evaluation = evaluate_trace(dir.path(), &TraceOptions::new(prd, tasks)).unwrap();
    assert!(
        evaluation.findings.iter().any(|finding| {
            finding.text.contains("REQ-X2-002")
                && finding.text.contains("REQ-<LETTERS>-<NNN>")
                && finding.text.contains("rename")
        }),
        "{:?}",
        evaluation.findings
    );
}

#[test]
fn cli_report_writer_preserves_the_exact_off_line() {
    let mut output = Vec::new();
    write_cli_report(&mut output, crate::command::workflow_gate::OFF_MESSAGE).unwrap();
    assert_eq!(
        output,
        b"gate_mode=off \xe2\x80\x94 nothing was evaluated; this is not a pass\n"
    );
}

#[test]
fn trace_gate_findings_retain_exact_backing_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(dir.path(), &["- REQ-X-001: valid."]);
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\nimplements: [REQ-X-999]\n```\n",
    );
    let task_path = tasks.join("TASK-X-001.md");
    let evaluation = evaluate_trace(dir.path(), &TraceOptions::new(prd.clone(), tasks)).unwrap();

    let phantom = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.contains("REQ-X-999"))
        .expect("phantom finding");
    assert_eq!(phantom.subject, "TASK-X-001");
    assert_eq!(phantom.source_path.as_deref(), Some(task_path.as_path()));

    let unclaimed = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.contains("REQ-X-001"))
        .expect("unclaimed finding");
    assert_eq!(unclaimed.source_path.as_deref(), Some(prd.as_path()));
}
