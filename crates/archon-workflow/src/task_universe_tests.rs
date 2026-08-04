use super::*;

/// A task file in the standard shape: a fenced YAML block declaring every
/// contract-bearing key.
///
/// These tests used to write files carrying bare `task_id:` / `depends_on:`
/// lines with no YAML block at all, which the old parser happily accepted by
/// scanning raw text. That is precisely the partial parse the parser now
/// refuses, so the fixtures are written the way real task files are written.
fn standard_task(task_id: &str, depends_on: &str, blocks: &str, body: &str) -> String {
    format!(
        "# {task_id}\n\n```yaml\ntask_id: {task_id}\ntitle: Fixture {task_id}\n\
         complexity: medium\nstatus: ready\ndepends_on: {depends_on}\nblocks: {blocks}\n\
         implements: []\nrequired_env_keys: []\nrequired_tools: []\n\
         deliverable_contracts: []\n```\n{body}"
    )
}

fn write_task(dir: &Path, file: &str, contents: &str) {
    fs::write(dir.join(file), contents).expect("write task file");
}

fn universe_at(dir: &Path) -> WorkflowResult<Option<WorkflowV2TaskUniverse>> {
    extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        dir.display()
    ))
}

#[test]
fn task_universe_resolves_canonical_and_alias_forms() {
    let universe = synthetic_universe(&[
        ("TASK-ALPHA-010", &["T010"], &[]),
        ("TASK-ALPHA-020", &["T020"], &["TASK-ALPHA-010"]),
    ]);

    for alias in ["TASK-ALPHA-010", "ALPHA-010", "T010"] {
        assert_eq!(
            universe.resolve_canonical_task_id(alias).unwrap(),
            "TASK-ALPHA-010"
        );
    }
}

#[test]
fn task_universe_rejects_ambiguous_short_aliases() {
    let universe = synthetic_universe(&[
        ("TASK-ALPHA-010", &["T010"], &[]),
        ("TASK-BETA-010", &["T010"], &[]),
    ]);

    let err = universe
        .resolve_canonical_task_id("T010")
        .expect_err("ambiguous short alias must fail");

    assert!(err.to_string().contains("ambiguous"));
    assert!(err.to_string().contains("TASK-ALPHA-010"));
    assert!(err.to_string().contains("TASK-BETA-010"));
}

#[test]
fn task_universe_computes_downstream_task_closure() {
    let universe = synthetic_universe(&[
        ("TASK-ALPHA-010", &["T010"], &[]),
        ("TASK-ALPHA-020", &["T020"], &["TASK-ALPHA-010"]),
        ("TASK-ALPHA-030", &["T030"], &["TASK-ALPHA-020"]),
        ("TASK-ALPHA-040", &["T040"], &[]),
    ]);

    assert_eq!(
        universe.downstream_task_closure("TASK-ALPHA-010"),
        ["TASK-ALPHA-010", "TASK-ALPHA-020", "TASK-ALPHA-030"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
}

#[test]
fn universe_comes_from_task_files_not_reducer_items() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path();
    write_task(
        dir,
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-001", "[]", "[]", ""),
    );
    write_task(
        dir,
        "TASK-TDL-010-dependent.md",
        &standard_task("TASK-TDL-010", "['TASK-TDL-001']", "[]", ""),
    );

    let universe = universe_at(dir).expect("extract").expect("universe");

    assert_eq!(
        universe.canonical_ids(),
        vec!["TASK-TDL-001".to_string(), "TASK-TDL-010".to_string()]
    );
    assert_eq!(
        universe.tasks[1].dependency_ids,
        vec!["TASK-TDL-001".to_string()]
    );
}

#[test]
fn task_universe_carries_authoritative_acceptance_criteria() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_task(
        temp.path(),
        "TASK-TDL-001-foundation.md",
        &standard_task(
            "TASK-TDL-001",
            "[]",
            "[]",
            "\n## Acceptance Criteria\n\n- First exact criterion.\n\
             - Second exact criterion with `literal` text.\n\n\
             ## Focused Tests\n\n- ignored test bullet\n",
        ),
    );

    let universe = universe_at(temp.path())
        .expect("extract")
        .expect("universe");

    assert_eq!(
        universe.tasks[0].acceptance_criteria,
        vec![
            "First exact criterion.".to_string(),
            "Second exact criterion with `literal` text.".to_string(),
        ]
    );
}

/// `## Adversarial Review Notes` is the task author's own list of falsification
/// hypotheses. It only became reachable when review moved to one reviewer per
/// task — a reducer holding every task at once has no use for it. The first
/// file mirrors a real task file (TASK-TDL-001), including the trailing
/// prior-run-findings block that must not leak into the notes; the second
/// declares none, and gets none — nothing is invented for a reviewer to chase.
#[test]
fn task_universe_carries_declared_adversarial_review_notes() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\n```yaml\ntask_id: TASK-TDL-001\ntitle: Foundation\ncomplexity: medium\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Acceptance Criteria\n\n- A criterion.\n\n## Adversarial Review Notes\n\n- Verify the task does not weaken native-candle enforcement.\n- Verify residual gaps fail closed.\n\n<!-- PRIOR-RUN-FINDINGS:BEGIN -->\n\n### Prior run `wf-ee4a92fc` (2026-07-28)\n\n- a prior finding bullet that is not a review note\n",
    )
    .expect("task");
    fs::write(
        temp.path().join("TASK-TDL-002-plain.md"),
        "# Plain\n\n```yaml\ntask_id: TASK-TDL-002\ntitle: Plain\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Acceptance Criteria\n\n- A criterion.\n",
    )
    .expect("task");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect("extract")
    .expect("universe");

    assert_eq!(
        universe.tasks[0].adversarial_review_notes,
        vec![
            "Verify residual gaps fail closed.".to_string(),
            "Verify the task does not weaken native-candle enforcement.".to_string(),
        ]
    );
    assert!(universe.tasks[1].adversarial_review_notes.is_empty());
}

#[test]
fn prd_task_references_must_have_matching_task_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let prd = temp.path().join("PRD.md");
    fs::write(&prd, "Acceptance references TASK-TDL-140.\n").expect("prd");
    let tasks = temp.path().join("tasks");
    fs::create_dir_all(&tasks).expect("tasks");
    write_task(
        &tasks,
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-001", "[]", "[]", ""),
    );

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {} and tasks at {}",
        prd.display(),
        tasks.display()
    ))
    .expect_err("unbacked PRD task reference must fail");

    assert!(err.to_string().contains("references TASK-TDL-140"));
}

#[test]
fn missing_authoritative_task_evidence_fails_for_decomposed_prd() {
    let err = extract_task_universe_for_generated_run(
        "Implement the decomposed PRD at /no/such/tasks/PRD-MISSING",
    )
    .expect_err("missing local evidence must fail");

    assert!(
        err.to_string()
            .contains("requires local TASK-*.md evidence")
    );
}

#[test]
fn invalid_task_id_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_task(
        temp.path(),
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-1", "[]", "[]", ""),
    );

    let err = universe_at(temp.path()).expect_err("invalid task id must fail");

    assert!(err.to_string().contains("invalid task_id"));
}

#[test]
fn dependency_cycles_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_task(
        temp.path(),
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-001", "[TASK-TDL-010]", "[]", ""),
    );
    write_task(
        temp.path(),
        "TASK-TDL-010-dependent.md",
        &standard_task("TASK-TDL-010", "[TASK-TDL-001]", "[]", ""),
    );

    let err = universe_at(temp.path()).expect_err("cycle must fail");

    assert!(err.to_string().contains("dependency cycle"));
}

// ---------------------------------------------------------------------------
// `blocks:` — the reverse edge
// ---------------------------------------------------------------------------

/// A file that expresses its ordering only through `blocks:` used to contribute
/// no edge at all, because nothing read the key. Its dependents then became
/// eligible immediately.
#[test]
fn a_blocks_declaration_alone_creates_the_dependency_edge() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_task(
        temp.path(),
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-001", "[]", "['TASK-TDL-010']", ""),
    );
    // Declares NO depends_on: the edge exists only in the blocker's file.
    write_task(
        temp.path(),
        "TASK-TDL-010-dependent.md",
        &standard_task("TASK-TDL-010", "[]", "[]", ""),
    );

    let universe = universe_at(temp.path())
        .expect("extract")
        .expect("universe");

    assert_eq!(
        universe.tasks[0].blocks_ids,
        vec!["TASK-TDL-010".to_string()]
    );
    assert_eq!(
        universe.tasks[1].dependency_ids,
        vec!["TASK-TDL-001".to_string()],
        "blocks must reconcile into the graph the runner schedules on"
    );
    assert_eq!(universe.downstream_task_closure("TASK-TDL-001").len(), 2);
}

/// The two directions agreeing is the normal case and must not double-count.
#[test]
fn both_directions_declared_reconcile_to_one_edge() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_task(
        temp.path(),
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-001", "[]", "['TASK-TDL-010']", ""),
    );
    write_task(
        temp.path(),
        "TASK-TDL-010-dependent.md",
        &standard_task("TASK-TDL-010", "['TASK-TDL-001']", "[]", ""),
    );

    let universe = universe_at(temp.path())
        .expect("extract")
        .expect("universe");

    assert_eq!(
        universe.tasks[1].dependency_ids,
        vec!["TASK-TDL-001".to_string()]
    );
}

/// Both orders claimed for one pair is unsatisfiable. Reported as the authoring
/// mistake it is, naming the file, rather than as an opaque two-cycle.
#[test]
fn a_task_that_both_blocks_and_depends_on_the_same_task_is_a_contradiction() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_task(
        temp.path(),
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-001", "['TASK-TDL-010']", "['TASK-TDL-010']", ""),
    );
    write_task(
        temp.path(),
        "TASK-TDL-010-dependent.md",
        &standard_task("TASK-TDL-010", "[]", "[]", ""),
    );

    let err = universe_at(temp.path()).expect_err("contradiction must fail");
    let message = err.to_string();
    assert!(message.contains("both blocks and depends_on"), "{message}");
    assert!(message.contains("TASK-TDL-010"), "{message}");
}

#[test]
fn two_tasks_each_claiming_to_block_the_other_is_a_contradiction() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_task(
        temp.path(),
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-001", "[]", "['TASK-TDL-010']", ""),
    );
    write_task(
        temp.path(),
        "TASK-TDL-010-dependent.md",
        &standard_task("TASK-TDL-010", "[]", "['TASK-TDL-001']", ""),
    );

    let err = universe_at(temp.path()).expect_err("mutual blocks must fail");
    assert!(
        err.to_string()
            .contains("each declare that they block the other"),
        "{err}"
    );
}

#[test]
fn an_unresolvable_blocks_reference_fails_naming_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_task(
        temp.path(),
        "TASK-TDL-001-foundation.md",
        &standard_task("TASK-TDL-001", "[]", "['TASK-TDL-999']", ""),
    );

    let err = universe_at(temp.path()).expect_err("dangling blocks must fail");
    assert!(
        err.to_string()
            .contains("unresolved blocks reference 'TASK-TDL-999'"),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// Loud failure on a non-conforming task file
// ---------------------------------------------------------------------------

fn parse_failure(body: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("TASK-DEMO-017-loud.md");
    fs::write(&path, body).expect("write task");
    parse_task_file(&path, body)
        .expect_err("a non-conforming task file must fail the run")
        .to_string()
}

/// The headline silence: a file with no YAML block used to parse into a record
/// with a filename-derived id, a heading-derived title, and no dependencies,
/// tools, env keys, or deliverables — and the run continued on an empty graph.
#[test]
fn a_task_file_with_no_yaml_block_fails_naming_the_path_and_the_required_keys() {
    let error = parse_failure("# Loud\n\ntask_id: TASK-DEMO-017\ndepends_on: []\n");
    assert!(error.contains("no fenced ```yaml task block"), "{error}");
    assert!(error.contains("TASK-DEMO-017-loud.md"), "{error}");
    for key in super::parsing::REQUIRED_TASK_KEYS {
        assert!(error.contains(key), "error must name '{key}': {error}");
    }
}

/// An unterminated fence is not a block. Parsing the truncated remainder would
/// be exactly the partial parse the parser exists to refuse.
#[test]
fn an_unterminated_yaml_fence_is_not_a_block() {
    let error = parse_failure("# Loud\n\n```yaml\ntask_id: TASK-DEMO-017\n");
    assert!(error.contains("no fenced ```yaml task block"), "{error}");
}

/// Malformed YAML used to be swallowed into an empty mapping.
#[test]
fn unparseable_yaml_fails_instead_of_becoming_an_empty_mapping() {
    let error = parse_failure("```yaml\ntask_id: [unclosed\n```\n");
    assert!(
        error.contains("could not parse the task YAML block"),
        "{error}"
    );
    assert!(error.contains("TASK-DEMO-017-loud.md"), "{error}");
}

/// A well-formed block that omits contract-bearing keys must name exactly the
/// ones it omitted — "declared empty" and "not declared" are different
/// statements and the parser refuses to guess which was meant.
#[test]
fn missing_required_keys_are_named_individually() {
    let error = parse_failure(concat!(
        "```yaml\n",
        "task_id: TASK-DEMO-017\n",
        "title: Loud\n",
        "complexity: small\n",
        "status: ready\n",
        "depends_on: []\n",
        "required_tools: []\n",
        "```\n"
    ));
    assert!(error.contains("is missing required key(s)"), "{error}");
    for missing in ["blocks", "required_env_keys", "deliverable_contracts"] {
        assert!(error.contains(missing), "{error}");
    }
    // Keys that WERE declared must not be reported as missing.
    assert!(!error.contains("complexity"), "{error}");
}

#[test]
fn malformed_deliverable_contract_fails_closed() {
    let error = parse_failure(concat!(
        "```yaml\n",
        "task_id: TASK-DEMO-017\n",
        "title: Loud\n",
        "complexity: small\n",
        "status: ready\n",
        "depends_on: []\n",
        "blocks: []\n",
        "implements: []\n",
        "required_env_keys: []\n",
        "required_tools: []\n",
        "deliverable_contracts:\n",
        "  - kind: required_universe_registry\n",
        "    artifact_path: 42\n",
        "```\n"
    ));
    assert!(
        error.contains("unreadable deliverable_contracts block"),
        "{error}"
    );
    assert!(error.contains("invalid type"), "{error}");
}

// The whole-fixture universe test that used to live here moved to
// `workflow_live_v2_prd_pipeline_tests::the_dependency_graph_honours_both_depends_on_and_blocks`,
// which makes the same 26-edge and downstream-closure assertions plus the
// wave, plan and per-task-input checks the same fixture supports. Two copies of
// it also pushed this file past the 500-line ceiling.

fn synthetic_universe(tasks: &[(&str, &[&str], &[&str])]) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks: tasks
            .iter()
            .map(
                |(canonical_task_id, aliases, dependency_ids)| WorkflowV2TaskUniverseTask {
                    canonical_task_id: (*canonical_task_id).to_string(),
                    aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
                    source_path: format!("tasks/{canonical_task_id}.md"),
                    dependency_ids: dependency_ids
                        .iter()
                        .map(|dependency| (*dependency).to_string())
                        .collect(),
                    ..Default::default()
                },
            )
            .collect(),
    }
}
