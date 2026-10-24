// One check's fault is that check's own result: every other check of the
// batch still runs on a clean site and gets its own verdict (a process per
// test, as a test runner gives each its own process; per-target results,
// as a keep-going build keeps them). Unix process groups, as the other
// acceptance_scratch targets.
#![cfg(unix)]

use archon_workflow::acceptance_scratch::faults::{CheckFault, check_fault};
use archon_workflow::acceptance_scratch::{ObservationResult, ScratchPolicy, observe_commands};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
use std::{collections::BTreeMap, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}

/// A repository, a project beside it and one accepted check per
/// `(command, cwd)`, ids AC-X-000, AC-X-001, ...
fn fixture(
    checks: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    ScratchPolicy,
    String,
    AcceptanceContract,
    Vec<FrozenCommandRef>,
) {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    let project = t.path().join("project");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(project.join("data")).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "test"]);
    std::fs::write(repo.join("input"), "source").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(project.join("data/value"), "before").unwrap();
    let policy = ScratchPolicy {
        repository: repo,
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: t.path().join("scratch"),
        project_inputs: vec!["data".into()],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: "/usr/bin:/bin:/usr/sbin:/sbin".into(),
        environment: BTreeMap::new(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 20,
        output_bytes: 2048,
        scratch_bytes: 16 * 1024 * 1024,
        build_cache: None,
    };
    let entries: Vec<_> = (checks.iter().enumerate())
        .map(|(n, (command, cwd))| serde_json::json!({"id":format!("AC-X-00{n}"),"criterion":"output correct","check":{"kind":"command","command":command,"cwd":cwd},"judgment":{"verdict":"accepted","counterexample":"incorrect output","reason":"checks output","host_call_id":"j"}}))
        .collect();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"p","digest":"d"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":entries})).unwrap();
    let refs = (checks.iter().enumerate())
        .map(|(n, (command, _))| FrozenCommandRef {
            acceptance_id: format!("AC-X-00{n}"),
            kind: AcceptanceCommandKind::Command,
            chain_digest: "chain".into(),
            command_digest: content_digest(command.as_bytes()),
        })
        .collect();
    (t, policy, commit, contract, refs)
}

async fn observe(checks: &[(&str, &str)]) -> (tempfile::TempDir, ObservationResult) {
    observe_with(checks, |_, _| {}).await
}

/// [`observe`] with the policy changed by `change` (given the fixture root).
async fn observe_with(
    checks: &[(&str, &str)],
    change: impl FnOnce(&Path, &mut ScratchPolicy),
) -> (tempfile::TempDir, ObservationResult) {
    let (t, mut policy, commit, contract, refs) = fixture(checks);
    change(t.path(), &mut policy);
    let result = observe_commands(
        &policy,
        &commit,
        &contract,
        "chain",
        &refs,
        &t.path().join("evidence"),
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
    .unwrap();
    (t, result)
}

fn is_checks_own(error: Option<&str>) -> bool {
    error.is_some_and(|error| matches!(check_fault(error), CheckFault::Check { .. }))
}

/// The other checks' verdicts, and an observation that is not void.
fn assert_siblings_judged(result: &ObservationResult, offender: usize) {
    assert!(
        result.operational_errors.is_empty() && result.teardown_verified,
        "one check's fault must not void the observation: {result:?}"
    );
    assert_eq!(result.checks.len(), 3, "{result:?}");
    for (index, check) in result.checks.iter().enumerate() {
        assert_eq!(check.acceptance_id, format!("AC-X-00{index}"), "{result:?}");
        if index != offender {
            assert_eq!(check.exit_code, Some(0), "sibling {index}: {check:?}");
            assert!(
                check.operational_error.is_none(),
                "sibling {index}: {check:?}"
            );
        }
    }
    let error = result.checks[offender].operational_error.as_deref();
    assert!(
        is_checks_own(error),
        "the offender's own finding: {error:?}"
    );
}

#[tokio::test]
async fn a_check_leaving_a_process_that_holds_its_pipes_voids_only_itself() {
    let outside = tempfile::tempdir().unwrap();
    let pid = outside.path().join("leaked-pid");
    // A descendant in its own session, still holding the check's pipes.
    let leaker = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); open(F, \">\", $ARGV[0]); print F $$; close F; sleep 8' '{}' &\nuntil [ -s '{}' ]; do sleep 0.01; done\ntest -f data/value",
        pid.display(),
        pid.display()
    );
    let (_scratch, result) = observe(&[
        ("test -f data/value", "project_root"),
        (&leaker, "project_root"),
        ("test -f input", "project_root"),
    ])
    .await;
    if let Ok(text) = std::fs::read_to_string(&pid) {
        let _ = Command::new("kill").args(["-9", text.trim()]).status();
    }
    assert_siblings_judged(&result, 1);
}

#[tokio::test]
async fn a_check_that_removes_its_build_output_link_is_the_checks_and_its_siblings_still_run() {
    let (_t, result) = observe(&[
        ("test -f data/value", "project_root"),
        ("rm -f target && test ! -e target", "project_root"),
        ("test -L target && test -f input", "project_root"),
    ])
    .await;
    assert_siblings_judged(&result, 1);
    let error = result.checks[1].operational_error.as_deref().unwrap();
    assert!(
        error.contains("target links"),
        "names what changed: {error}"
    );
}

#[tokio::test]
async fn a_check_that_changes_its_source_runs_alone_and_the_next_check_sees_the_restored_source() {
    let (_t, result) = observe(&[
        ("test -f data/value", "project_root"),
        (
            "printf modified > input && grep -q modified input",
            "project_root",
        ),
        ("grep -q source input", "project_root"),
    ])
    .await;
    assert_siblings_judged(&result, 1);
}

#[tokio::test]
async fn a_sibling_path_that_reaches_the_project_root_is_only_that_checks_finding() {
    let (_t, result) = observe(&[
        ("test -f data/value", "project_root"),
        ("test -d ../project", "repo_root"),
        ("test -f input", "project_root"),
    ])
    .await;
    assert_siblings_judged(&result, 1);
    let error = result.checks[1].operational_error.as_deref().unwrap();
    assert!(error.contains("project root"), "{error}");
}

/// R5-1: with the warm build cache, a process the check left (holding and
/// still writing into the build output it was given) cannot reach the next check's site
/// or build output.
#[tokio::test]
async fn a_process_a_check_left_cannot_write_into_the_next_checks_build_output() {
    let outside = tempfile::tempdir().unwrap();
    let pid = outside.path().join("writer-pid");
    let writer = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); open(F, \">\", $ARGV[0]); print F $$; close F; open(G, \">>\", \"$ARGV[1]/held\"); for (1..300) {{ syswrite(G, \"x\"); open(H, \">>\", \"$ARGV[1]/leaked\"); close H; select(undef, undef, undef, 0.05) }}' '{}' \"$CARGO_TARGET_DIR\" </dev/null >/dev/null 2>&1 &\nuntil [ -s '{}' ]; do sleep 0.01; done\ntest -f data/value",
        pid.display(),
        pid.display()
    );
    let (_t, result) = observe_with(
        &[
            ("test -f data/value", "project_root"),
            (&writer, "project_root"),
            (
                "test ! -e \"$CARGO_TARGET_DIR/held\" && sleep 1 && test ! -e \"$CARGO_TARGET_DIR/leaked\"",
                "project_root",
            ),
        ],
        |root, policy| policy.build_cache = Some(root.join("cache")),
    )
    .await;
    if let Ok(text) = std::fs::read_to_string(&pid) {
        let _ = Command::new("kill").args(["-9", text.trim()]).status();
    }
    assert_siblings_judged(&result, 1);
}

/// The checks still to run after a site rebuild failed: each is
/// host-unproven, never the observation's failure.
fn assert_host_unproven(check: &archon_workflow::acceptance_scratch::CheckResult, cause: &str) {
    let error = check.operational_error.as_deref().unwrap_or_default();
    assert_eq!(check.exit_code, None, "never run: {check:?}");
    assert!(error.contains(cause), "{error}");
    assert_eq!(check_fault(error), CheckFault::Host, "{error}");
}

/// R6 G1: check 2's own fault forces a rebuild, and the rebuilt site
/// differs from the batch's first one on a part that does not depend on
/// its path (a tool check 2 changed). Check 1 keeps its verdict, check 2
/// its own fault, the checks after it are host-unproven, and the
/// observation stands (no operational error voids it). A check refused
/// before any site existed keeps its refusal.
#[tokio::test]
async fn a_failed_site_rebuild_leaves_only_the_later_checks_host_unproven() {
    let tools = tempfile::tempdir().unwrap();
    let tool = tools.path().join("python3");
    std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
    let change = format!(
        "printf changed >> '{}' && rm -f target && test -f data/value",
        tool.display()
    );
    let toolchain = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", tools.path().display());
    let (_t, result) = observe_with(
        &[
            ("test -f data/value", "project_root"),
            (&change, "project_root"),
            ("test -f data/value", "project_root"),
            ("test -d ../project", "repo_root"),
        ],
        |_, policy| policy.toolchain_path = toolchain,
    )
    .await;
    assert!(
        result.operational_errors.is_empty() && result.teardown_verified,
        "a failed rebuild must not void the observation: {result:?}"
    );
    assert!(result.live_roots_unchanged, "{result:?}");
    assert_eq!(result.checks.len(), 4, "{result:?}");
    assert_eq!(result.checks[0].exit_code, Some(0), "{result:?}");
    assert!(result.checks[0].operational_error.is_none(), "{result:?}");
    let own = result.checks[1].operational_error.as_deref();
    assert!(is_checks_own(own), "check 2 keeps its own fault: {own:?}");
    assert_host_unproven(&result.checks[2], "site rebuild failed");
    assert_host_unproven(
        &result.checks[2],
        "differs from the batch's first site (tools",
    );
    let refused = result.checks[3].operational_error.as_deref().unwrap();
    assert!(
        is_checks_own(Some(refused)) && refused.contains("project root"),
        "a refusal made before any site existed is kept: {refused}"
    );
}

/// A sibling directory beside the fixture's project, holding `file` of
/// `bytes` bytes (sparse), readable only when `readable`.
fn sibling(root: &Path, name: &str, bytes: u64, readable: bool) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(root.join(name)).unwrap();
    let file = root.join(name).join("file");
    std::fs::File::create(&file)
        .unwrap()
        .set_len(bytes)
        .unwrap();
    let mode = if readable { 0o644 } else { 0o000 };
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// R6 G2: a sibling only one check names, too large to copy, is that
/// check's own fault; the other checks run.
#[tokio::test]
async fn a_sibling_too_large_for_the_site_is_only_its_checks_fault() {
    let (_t, result) = observe_with(
        &[
            ("test -f data/value", "project_root"),
            ("test -d ../large", "project_root"),
            ("test -f input", "project_root"),
        ],
        |root, _| sibling(root, "large", 64 * 1024 * 1024, true),
    )
    .await;
    assert_siblings_judged(&result, 1);
    let error = result.checks[1].operational_error.as_deref().unwrap();
    assert!(error.contains("scratch size limit"), "{error}");
}

/// R6 G2: a sibling only one check names, which the host cannot read, leaves
/// that check alone host-unproven; the other checks run.
#[tokio::test]
async fn a_sibling_the_host_cannot_read_leaves_only_its_check_host_unproven() {
    let (t, result) = observe_with(
        &[
            ("test -f data/value", "project_root"),
            ("test -d ../locked", "project_root"),
            ("test -f input", "project_root"),
        ],
        |root, _| sibling(root, "locked", 8, false),
    )
    .await;
    let file = t.path().join("locked/file");
    let readable = std::os::unix::fs::PermissionsExt::from_mode(0o644);
    std::fs::set_permissions(file, readable).unwrap();
    assert!(result.operational_errors.is_empty(), "{result:?}");
    for index in [0, 2] {
        assert_eq!(result.checks[index].exit_code, Some(0), "{result:?}");
    }
    assert_host_unproven(&result.checks[1], "could not be copied");
}

/// R6 G2: a sibling several checks share that cannot be copied leaves each
/// of those checks host-unproven, even when its cause is in the path; the
/// checks that do not name it run.
#[tokio::test]
async fn a_shared_sibling_that_cannot_be_copied_leaves_each_of_its_checks_host_unproven() {
    let (_t, result) = observe_with(
        &[
            ("test -f data/value", "project_root"),
            ("test -d ../large", "project_root"),
            ("test -f ../large/file", "project_root"),
            ("test -f input", "project_root"),
        ],
        |root, _| sibling(root, "large", 64 * 1024 * 1024, true),
    )
    .await;
    assert!(result.operational_errors.is_empty(), "{result:?}");
    for index in [0, 3] {
        assert_eq!(result.checks[index].exit_code, Some(0), "{result:?}");
    }
    for index in [1, 2] {
        assert_host_unproven(&result.checks[index], "could not be copied");
    }
}

/// R6 G5: without a build cache, a writer check 1 left running (its fault)
/// cannot write into the next check's project, repository, build output,
/// home, temporary or build-tool home directory: the restored site is a new root.
#[tokio::test]
async fn a_writer_a_check_left_cannot_reach_the_next_checks_site_without_a_build_cache() {
    let outside = tempfile::tempdir().unwrap();
    let pid = outside.path().join("writer-pid");
    let dirs = "\"$PWD\" \"$ARCHON_SCRATCH_SITE_ROOT/repo\" \"$CARGO_TARGET_DIR\" \"$HOME\" \"$TMPDIR\" \"$CARGO_HOME\"";
    let writer = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); open(F, \">\", shift); print F $$; close F; open(G, \">>\", \"$ARGV[3]/held\"); for (1..300) {{ syswrite(G, \"x\"); for my $d (@ARGV) {{ open(H, \">>\", \"$d/leak\"); close H }} select(undef, undef, undef, 0.05) }}' '{}' {dirs} </dev/null >/dev/null 2>&1 &\nuntil [ -s '{}' ]; do sleep 0.01; done\ntest -f data/value",
        pid.display(),
        pid.display()
    );
    let clean = format!(
        "sleep 1 && for d in {dirs}; do test ! -e \"$d/leak\" && test ! -e \"$d/held\" || exit 1; done"
    );
    let (_t, result) = observe(&[
        (&writer, "project_root"),
        (&clean, "project_root"),
        ("test -f data/value", "project_root"),
    ])
    .await;
    if let Ok(text) = std::fs::read_to_string(&pid) {
        let _ = Command::new("kill").args(["-9", text.trim()]).status();
    }
    assert_siblings_judged(&result, 0);
}
