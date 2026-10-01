//! Batch E: a failing check reaches the tasks that can write its fix; Batch
//! E2: never through a warning, and never outside the plan's scope roots.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::acceptance_regression::AcceptanceRegressionV1;
use crate::v2::acceptance_signals::failure_locations;
use crate::v2::acceptance_stage::{AcceptanceCheckRecordV1, AcceptanceCheckStatus};

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "pkg/core/engine.rs",
        "pkg/core/loose.rs",
        "pkg/core/sealed.rs",
        "pkg/cli/args.rs",
        "docs/notes.md",
        // `pkg/` is a package: the product area the tasks' declarations cover.
        "pkg/Cargo.toml",
        // The harness in the same repository, under the root package only.
        "Cargo.toml",
        "harness/gate.rs",
    ] {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    dir
}

fn task(id: &str, owns: &[&str], forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        implements: if id == "TASK-IMPL" {
            vec!["AC-1".into()]
        } else {
            Vec::new()
        },
        ..Default::default()
    }
}

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            // Implements the check, and may not touch the sealed module.
            task("TASK-IMPL", &["pkg/cli/args.rs"], &["`pkg/core/sealed.rs`"]),
            // Owns the engine module the failure points into.
            task("TASK-OWNER", &["pkg/core/engine.rs"], &[]),
        ],
    }
}

fn failing(stderr: &str, owners: &[&str]) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: "AC-1".into(),
        criterion: "the command succeeds".into(),
        kind: "command".into(),
        status: AcceptanceCheckStatus::Failed,
        exit_code: Some(1),
        operational_error: None,
        owning_tasks: owners.iter().map(|s| s.to_string()).collect(),
        stdout_tail: String::new(),
        stderr_tail: stderr.into(),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: None,
        blocked: None,
    }
}

fn round(checks: Vec<AcceptanceCheckRecordV1>) -> AcceptanceRoundRecordV1 {
    AcceptanceRoundRecordV1 {
        schema_version: 1,
        run_id: "run".into(),
        call_id: "acceptance-contract-run-1".into(),
        round: 1,
        attempt: 1,
        max_rounds: 3,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks,
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: false,
    }
}

#[test]
fn a_failure_in_another_tasks_file_routes_that_task_too() {
    let dir = repo();
    let root = dir.path();
    let stderr = format!(
        "thread 'main' panicked at {}/pkg/core/engine.rs:41:9:\nrejected input\n",
        root.display()
    );
    let mut record = round(vec![failing(&stderr, &["TASK-IMPL"])]);
    route_failures(Some(&universe()), root, &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.implicated_files, ["pkg/core/engine.rs"]);
    assert_eq!(routing.writer_tasks, ["TASK-OWNER"]);
    assert!(routing.granted_files.is_empty());
    assert!(routing.describe().contains("routed also to TASK-OWNER"));
}

#[test]
fn an_unowned_file_is_granted_and_a_forbidden_or_protected_one_is_not() {
    let dir = repo();
    let root = dir.path();
    let stderr = "pkg/core/loose.rs:7: error: rejected\npkg/core/sealed.rs:3: error: sealed\n\
                  docs/notes.md:1: error: stale\n";
    let mut record = round(vec![failing(stderr, &["TASK-IMPL"])]);
    route_failures(Some(&universe()), root, &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.granted_files, ["pkg/core/loose.rs"]);
    assert!(routing.writer_tasks.is_empty(), "{routing:?}");
    let unwritable: Vec<&str> = routing.unwritable.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(unwritable, ["docs/notes.md", "pkg/core/sealed.rs"]);
}

#[test]
fn an_unimplemented_check_is_remediable_through_the_owner_of_its_broken_file() {
    let dir = repo();
    let root = dir.path();
    let mut check = failing("", &[]);
    check.regressed_by = Some(AcceptanceRegressionV1 {
        held_at: "a".into(),
        landing_commit: "b".into(),
        landing_stage: "s".into(),
        tasks: Vec::new(),
        changed_files: vec!["pkg/core/engine.rs".into(), "gone.rs".into()],
        probed_as: None,
    });
    let mut record = round(vec![check]);
    assert!(!record.has_remediable_failures());
    route_failures(Some(&universe()), root, &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    // Batch J: a path the landing deleted is implicated too.
    assert_eq!(routing.implicated_files, ["pkg/core/engine.rs", "gone.rs"]);
    assert_eq!(routing.writer_tasks, ["TASK-OWNER"]);
    assert!(record.has_remediable_failures());
}

#[test]
fn locations_are_read_from_the_end_bounded_and_only_as_repository_files() {
    let dir = repo();
    let root = dir.path();
    let text = "  at pkg/cli/args.rs:1\n  at https://x.test/a.rs:2 ../escape.rs:3 /abs/elsewhere.rs:4\n\
                  at pkg/core/engine.rs:9:1 pkg/core/engine.rs:10 `pkg/core/loose.rs:2`,";
    assert_eq!(
        failure_locations(text, root, 6),
        ["pkg/core/loose.rs", "pkg/core/engine.rs", "pkg/cli/args.rs"]
    );
    assert_eq!(failure_locations(text, root, 1), ["pkg/core/loose.rs"]);
    // A path with no line location is not a failure location.
    assert!(failure_locations("  at pkg/core/engine.rs", root, 6).is_empty());
    // A passing check or no universe routes nothing.
    let panic = "thread 'main' panicked at pkg/core/engine.rs:1:1:";
    let mut record = round(vec![failing(panic, &[])]);
    record.checks[0].status = AcceptanceCheckStatus::Passed;
    route_failures(Some(&universe()), root, &[], &mut record);
    assert!(record.checks[0].routing.is_none());
    let mut record = round(vec![failing(panic, &[])]);
    route_failures(None, root, &[], &mut record);
    assert!(record.checks[0].routing.is_none());
}

#[test]
fn the_checks_own_source_is_never_routed_or_granted_and_scratch_paths_resolve() {
    let dir = repo();
    let root = dir.path();
    let stderr = "thread panicked at /scratch/observation-1/repo/pkg/core/loose.rs:9:1\n\
                     at /scratch/observation-1/repo/pkg/core/engine.rs:3\n";
    let check = failing(stderr, &["TASK-IMPL"]);
    // The check runs the `loose` target: that file is the check itself.
    let universe = universe();
    let texts = TaskTexts::read(&universe, root);
    let scope = PlanScopeRoots::of(&universe, root);
    let routing = route_check(
        &universe,
        root,
        &texts,
        &scope,
        &OwnershipMap::new(),
        &check,
        "runner test --target loose -q",
    )
    .expect("routed");

    assert_eq!(routing.implicated_files, ["pkg/core/engine.rs"]);
    assert_eq!(routing.writer_tasks, ["TASK-OWNER"]);
    assert!(routing.granted_files.is_empty());
    assert_eq!(routing.unwritable.len(), 1);
    assert_eq!(routing.unwritable[0].0, "pkg/core/loose.rs");
    assert!(routing.unwritable[0].1.contains("the check's own source"));
}

#[test]
fn only_what_the_command_runs_is_its_own_source() {
    for (command, own) in [
        ("python3 pkg/core/loose.rs --strict", true),
        ("./pkg/core/loose.rs", true),
        ("LANG=C runner --target loose", true),
        ("runner --target=loose", false),
        ("grep -n needle pkg/core/loose.rs", false),
        ("runner test --lib", false),
        ("runner test loose_case", false),
    ] {
        assert_eq!(own_source("pkg/core/loose.rs", command), own, "{command}");
    }
}

#[test]
fn windows_drive_and_verbatim_diagnostics_route_existing_repository_files() {
    let dir = repo();
    for location in [
        r"C:\scratch\pkg\core\engine.rs:41:9:",
        "C:/scratch/pkg/core/engine.rs:41:9:",
        r"\\?\C:\scratch\pkg\core\engine.rs:41:9:",
    ] {
        let panic = format!("thread 'main' panicked at {location}");
        let mut record = round(vec![failing(&panic, &["TASK-IMPL"])]);
        route_failures(Some(&universe()), dir.path(), &[], &mut record);
        let routing = record.checks[0]
            .routing
            .as_ref()
            .expect("Windows location routes");
        assert_eq!(
            routing.implicated_files,
            ["pkg/core/engine.rs"],
            "{location}"
        );
        assert_eq!(routing.writer_tasks, ["TASK-OWNER"], "{location}");
    }
    for location in [
        "C:/scratch/pkg/core/engine.rs",
        "C:/scratch/pkg/core/engine.rs:word",
    ] {
        let panic = format!("thread 'main' panicked at {location}");
        assert!(
            failure_locations(&panic, dir.path(), 6).is_empty(),
            "{location}"
        );
    }
}

/// The live wf-0ddadd81 shape: the check built the binary (a page of rustc
/// warnings, a linker message) and then failed with a location-less error.
/// Batch E implicated and granted every warning's file, harness included.
#[test]
fn warning_blocks_implicate_nothing_and_grant_nothing() {
    let dir = repo();
    let root = dir.path();
    let stderr = "warning: function `freeze_acceptance` is never used\n \
                  --> harness/gate.rs:5:21\n  |\n5 | pub(crate) async fn freeze_acceptance(\n  \
                  |                     ^^^^^^^^^^^^^^^^^\n\n\
                  warning: unused import: `super::migration`\n  \
                  --> pkg/core/loose.rs:12:5\n   |\n12 | use super::migration;\n   \
                  |     ^^^^^^^^^^^^^^^^\n   |\n   = note: `#[warn(unused_imports)]` on by default\n\n\
                  harness/gate.rs:9:1: warning: unused variable\n\
                  warning: linker stderr: ld: __eh_frame section too large\n  |\n  \
                  = note: `#[warn(linker_messages)]` on by default\n\n\
                  Error: unknown asset_class `unknown`\n";
    let mut record = round(vec![failing(stderr, &["TASK-IMPL"])]);
    route_failures(Some(&universe()), root, &[], &mut record);
    assert_eq!(record.checks[0].routing, None);
}

#[test]
fn an_unowned_file_outside_the_scope_roots_is_never_granted() {
    let dir = repo();
    let root = dir.path();
    // A real failure location, in the harness: no task declares it and it is
    // not protected, but it is not the task set's product area.
    let stderr = "thread 'main' panicked at harness/gate.rs:41:9:\nboom\n";
    let mut record = round(vec![failing(stderr, &["TASK-IMPL"])]);
    route_failures(Some(&universe()), root, &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("recorded");
    assert_eq!(routing.implicated_files, ["harness/gate.rs"]);
    assert!(routing.granted_files.is_empty(), "{routing:?}");
    assert!(routing.writer_tasks.is_empty(), "{routing:?}");
    assert_eq!(routing.unwritable.len(), 1);
    assert_eq!(routing.unwritable[0].0, "harness/gate.rs");
    assert!(
        routing.unwritable[0].1.contains("scope roots"),
        "{routing:?}"
    );
    // With no implementer, a task whose text names the harness file is not
    // routed to it either.
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(root.join("tasks/TASK-OWNER.md"), "see harness/gate.rs\n").unwrap();
    let mut record = round(vec![failing(stderr, &[])]);
    route_failures(Some(&universe()), root, &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("recorded");
    assert!(routing.writer_tasks.is_empty() && routing.granted_files.is_empty());
}

#[test]
fn an_unowned_file_inside_the_scope_roots_is_granted_from_a_panic_location() {
    let dir = repo();
    let root = dir.path();
    let stderr = "warning: unused\n --> harness/gate.rs:2:1\n\n\
                  thread 'main' panicked at pkg/core/loose.rs:7:5:\nassertion failed\n\
                  note: run with `RUST_BACKTRACE=1` environment variable\n";
    let mut record = round(vec![failing(stderr, &["TASK-IMPL"])]);
    route_failures(Some(&universe()), root, &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.implicated_files, ["pkg/core/loose.rs"]);
    assert_eq!(routing.granted_files, ["pkg/core/loose.rs"]);
    assert!(routing.unwritable.is_empty(), "{routing:?}");
}

/// Batch J: the blamed landing's own changes are its authors' to restore,
/// inside the plan's scope roots or not (the write boundary admitted them
/// when they landed), a file it deleted included. A file read from the
/// check's OUTPUT stays bounded by the roots (E2, tested above).
#[test]
fn a_blamed_landings_own_files_are_granted_to_its_authors_even_outside_the_scope_roots() {
    let dir = repo();
    let mut check = failing("", &["TASK-IMPL"]);
    check.regressed_by = Some(AcceptanceRegressionV1 {
        held_at: "a".into(),
        landing_commit: "b".into(),
        landing_stage: "s".into(),
        tasks: vec!["TASK-OWNER".into()],
        changed_files: vec![
            "harness/gate.rs".into(),
            "pkg/core/loose.rs".into(),
            "pkg/core/removed.rs".into(),
            "../escape.rs".into(),
        ],
        probed_as: None,
    });
    let mut record = round(vec![check]);
    route_failures(Some(&universe()), dir.path(), &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(
        routing.granted_files,
        [
            "harness/gate.rs",
            "pkg/core/loose.rs",
            "pkg/core/removed.rs"
        ],
        "{routing:?}"
    );
    assert!(routing.unwritable.is_empty(), "{routing:?}");
}

/// Batch J: a file the blamed landing changed is its authors' to restore.
/// An OWNER forbidding it (live: the owner forbade `src/command/**`) never
/// withholds it; the authors forbidding it themselves does.
#[test]
fn a_landings_own_file_is_granted_to_its_author_though_an_owner_forbids_it() {
    let dir = repo();
    let regression = AcceptanceRegressionV1 {
        held_at: "a".into(),
        landing_commit: "b".into(),
        landing_stage: "review-remediate-task-owner-1-9".into(),
        tasks: vec!["TASK-OWNER".into()],
        changed_files: vec!["pkg/core/sealed.rs".into()],
        probed_as: None,
    };
    // No `path:line`: the landing is the route.
    let mut check = failing("Error: unknown asset_class `unknown`\n", &["TASK-IMPL"]);
    check.regressed_by = Some(regression);
    let mut record = round(vec![check]);
    route_failures(Some(&universe()), dir.path(), &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.implicated_files, ["pkg/core/sealed.rs"]);
    assert_eq!(routing.granted_files, ["pkg/core/sealed.rs"], "{routing:?}");
    assert!(routing.unwritable.is_empty(), "{routing:?}");
    // Forbidden to the author itself: not granted, and the reason says so.
    let mut universe = universe();
    universe.tasks[1].files_forbidden_to_change = vec!["`pkg/core/sealed.rs`".into()];
    record.checks[0].routing = None;
    route_failures(Some(&universe), dir.path(), &[], &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert!(routing.granted_files.is_empty(), "{routing:?}");
    assert!(
        routing.unwritable[0]
            .1
            .contains("forbidden to TASK-OWNER, whose landing changed it"),
        "{routing:?}"
    );
}

/// Batch J (d): a failed check no unit can fix is marked blocked with its
/// rule and never makes a round remediable; one whose search was only cut
/// short still goes to its owners.
#[test]
fn a_check_no_unit_can_fix_is_blocked_with_its_rule() {
    use crate::v2::acceptance_regression::RegressionSearchV1;
    let dir = repo();
    let root = dir.path();
    // Nothing routes it: no owner, no landing, no file.
    let mut check = failing("Error: boom\n", &[]);
    check.regression_search = Some(RegressionSearchV1::not_searched("no scratch policy"));
    let mut record = round(vec![check]);
    route_failures(Some(&universe()), root, &[], &mut record);
    mark_blocked(&mut record);
    let rule = record.checks[0].blocked.clone().expect("blocked");
    assert!(
        rule.contains("no task's `implements` names it") && rule.contains("no scratch policy"),
        "{rule}"
    );
    assert!(!record.has_remediable_failures());
    assert_eq!(record.blocked_checks().len(), 1);
    // Owned, but it never held in the run and fails only in the harness.
    let panic = "thread 'main' panicked at harness/gate.rs:41:9:\nboom\n";
    let never = RegressionSearchV1 {
        never_held: true,
        observed: 9,
        points: 9,
        note: "it never held in this run".into(),
        probed_as: None,
    };
    let mut check = failing(panic, &["TASK-IMPL"]);
    check.regression_search = Some(never.clone());
    let mut record = round(vec![check]);
    route_failures(Some(&universe()), root, &[], &mut record);
    mark_blocked(&mut record);
    let rule = record.checks[0].blocked.clone().expect("blocked");
    assert!(
        rule.contains("never held") && rule.contains("harness/gate.rs"),
        "{rule}"
    );
    assert!(!record.has_remediable_failures());
    // The same failure whose search was cut short goes to its owner.
    let mut check = failing(panic, &["TASK-IMPL"]);
    check.regression_search = Some(RegressionSearchV1::not_searched("budget ran out"));
    let mut record = round(vec![check]);
    route_failures(Some(&universe()), root, &[], &mut record);
    mark_blocked(&mut record);
    assert_eq!(record.checks[0].blocked, None);
    assert!(record.has_remediable_failures());
    // A check that could not be evaluated is never blocked: it did not run.
    let mut check = failing("", &[]);
    check.status = AcceptanceCheckStatus::Error;
    let mut record = round(vec![check]);
    mark_blocked(&mut record);
    assert_eq!(record.checks[0].blocked, None);
}
