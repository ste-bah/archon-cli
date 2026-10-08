//! The review fixes, each through a real probe on real trees.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering::SeqCst;

use archon_workflow::task_set_contract::{JudgeDecision, TrustedCwd};

use super::probe_tests::{git, trees, trees_in};
use super::{Baseline, ExecutabilityProbe, HOST_UNPROVEN, HostProbe};

const PROJECT: TrustedCwd = TrustedCwd::ProjectRoot;
const REPO: TrustedCwd = TrustedCwd::RepoRoot;

/// The round's own site (live, no policy) held to `commit`.
fn round_at(
    trees: &super::probe_tests::Trees,
    commit: &str,
    copies: &std::path::Path,
) -> HostProbe {
    HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    )
    .with_baseline(Baseline {
        commit: commit.to_string(),
        repository: trees.repo.clone(),
    })
    .with_copy_parent(copies.to_path_buf())
}

/// Fix 1: moving a check's own script, its `cd` target or its manifest, or
/// a run that only crashes once its data is gone, never counts as "can
/// fail": each such check passing before any implementation goes back to
/// its author.
#[tokio::test]
async fn what_makes_a_check_run_never_proves_it_can_fail() {
    let crashes_without_data =
        "python3 -c 'import os\nif not os.path.exists(\"data/state.txt\"): undefined_helper()\n'";
    let trees = trees(&[
        ("AC-1-001", "sh check.sh", PROJECT),
        (
            "AC-1-002",
            "cd sub && python3 -c 'import sys; sys.exit(0)'",
            PROJECT,
        ),
        ("AC-1-003", "grep -q name Cargo.toml", PROJECT),
        ("AC-1-004", crashes_without_data, PROJECT),
    ]);
    let project = trees.set.project.path();
    std::fs::write(project.join("check.sh"), "exit 0\n").unwrap();
    std::fs::create_dir_all(project.join("sub")).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let ids: Vec<&String> = findings.keys().collect();
    assert_eq!(
        ids,
        vec!["AC-1-001", "AC-1-002", "AC-1-003", "AC-1-004"],
        "{findings:?} {:?}",
        probe.take_diagnostics()
    );
    assert!(
        findings["AC-1-004"].contains("only crashed"),
        "{}",
        findings["AC-1-004"]
    );
    trees.assert_live_untouched(copies.path());
}

/// Fix 3: every mutated check runs in its own fresh copy.
#[tokio::test]
async fn each_mutated_check_gets_its_own_copy() {
    let trees = trees(&[
        ("AC-3-001", "grep -q ready data/state.txt", PROJECT),
        ("AC-3-002", "grep -q ready src/state.txt", REPO),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(
        findings.is_empty(),
        "both are regression guards: {findings:?}"
    );
    assert_eq!(
        probe.copies_made.load(SeqCst),
        3,
        "the baseline's copy and one per mutation"
    );
    trees.assert_live_untouched(copies.path());
}

/// Fix 4: a tracked symlink to a directory outside every copy is never
/// followed: the live file it reaches is never moved, and the check is not
/// proven by it.
#[cfg(unix)]
#[tokio::test]
async fn an_input_behind_a_tracked_symlink_never_proves_a_check() {
    let trees = trees(&[("AC-4-001", "grep -q ready ext/state.txt", REPO)]);
    let live = tempfile::tempdir().unwrap();
    std::fs::write(live.path().join("state.txt"), "ready").unwrap();
    std::os::unix::fs::symlink(live.path(), trees.repo.join("ext")).unwrap();
    git(&trees.repo, &["add", "ext"]);
    git(&trees.repo, &["commit", "-qm", "link"]);
    let head = git(&trees.repo, &["rev-parse", "HEAD"]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &head, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(
        findings
            .get("AC-4-001")
            .is_some_and(|f| f.contains("names none that exists there")),
        "{findings:?} {:?}",
        probe.take_diagnostics()
    );
    assert_eq!(
        std::fs::read_to_string(live.path().join("state.txt")).unwrap(),
        "ready"
    );
}

/// Fix 5: a verdict a freeze observed is reused only while the project data
/// it ran on is unchanged.
#[tokio::test]
async fn a_remembered_verdict_never_outlives_the_project_data_it_ran_on() {
    let trees = trees(&[("AC-5-001", "grep -rqF \"fl\"\"ag-on-7d\" .", PROJECT)]);
    let copies = tempfile::tempdir().unwrap();
    let freeze = || {
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
            .with_copy_parent(copies.path().to_path_buf())
    };
    let first = freeze()
        .script_defects(&trees.contract(), &trees.ids())
        .await;
    assert!(
        first.is_empty(),
        "it fails before the data exists: {first:?}"
    );
    std::fs::write(trees.set.project.path().join("data/flag"), "flag-on-7d").unwrap();
    let second = freeze()
        .script_defects(&trees.contract(), &trees.ids())
        .await;
    assert!(
        second
            .get("AC-5-001")
            .is_some_and(|f| f.contains(super::CANNOT_FAIL)),
        "the new data is probed, not the old verdict reused: {second:?}"
    );
}

/// Fix 6: a check the host cannot run stops the re-author operationally;
/// the author is never asked to change it.
#[tokio::test]
async fn a_host_failure_is_never_fed_to_the_author() {
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    use crate::command::workflow_task_set::reauthor::{AuthorScope, ReauthorGate, reauthor};
    let trees = trees(&[("AC-6-001", "test -f nothing-yet", REPO)]);
    let mut contract = trees.contract();
    contract.acceptance[0].judgment.verdict = JudgeDecision::Refuted;
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path()).with_injected_failures(100);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f feature.txt && test -s feature.txt"),
        |_, _| true,
    );
    let scope =
        AuthorScope::for_task_set(trees.set.project.path(), &trees.set.tasks, &trees.set.prd);
    let seeds = BTreeMap::new();
    let error = reauthor(
        &client,
        &contract,
        &trees.ids(),
        &scope,
        "sonnet",
        &ReauthorGate {
            probe: &probe,
            seeds: &seeds,
        },
    )
    .await
    .expect_err("an unprovable check is never published");
    assert!(
        error.downcast_ref::<super::HostUnproven>().is_some(),
        "{error:#}"
    );
    assert!(format!("{error:#}").contains(HOST_UNPROVEN), "{error:#}");
    assert_eq!(client.authored(), 1, "the author is not asked again");
    let prompt = client.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains("never its own script"),
        "prompt wording: {prompt}"
    );
}

/// Fix 7: a configured policy that cannot be captured runs nothing, in any
/// copy; the checks are unproven, never published.
#[tokio::test]
async fn a_broken_scratch_policy_runs_nothing() {
    let trees = trees(&[(
        "AC-7-001",
        "touch executed-marker && test -f built.txt",
        PROJECT,
    )]);
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        "[workflow.acceptance_execution]\nrepository=\"/nonexistent/archon-probe-repo\"\nscratch_parent=\"/nonexistent/scratch\"\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin\"\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "{findings:?}");
    let unproven = probe.take_unproven();
    assert!(
        unproven
            .get("AC-7-001")
            .is_some_and(|why| why.contains("could not be captured")),
        "{unproven:?}"
    );
    assert_eq!(probe.copies_made.load(SeqCst), 0, "no copy was even made");
    trees.assert_live_untouched(copies.path());
}

/// Fix 7: a check naming a live root by its absolute path is never run.
#[tokio::test]
async fn a_check_naming_a_live_root_is_refused_unrun() {
    let trees = trees(&[("AC-7-002", "placeholder", PROJECT)]);
    let live = trees
        .set
        .project
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let mut contract = trees.contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut contract.acceptance[0].check
    {
        *command = format!(
            "touch {}/executed-marker && test -f built.txt",
            live.display()
        );
    }
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &trees.base, copies.path());
    let findings = probe.script_defects(&contract, &trees.ids()).await;
    // Issue 366: the author step's entry validator gives this same text.
    assert_eq!(
        findings.get("AC-7-002"),
        Some(&crate::command::workflow_task_set::live_root::live_root_finding("AC-7-002", &live)),
        "{findings:?}"
    );
    trees.assert_live_untouched(copies.path());
}

/// Minors: a stale copy a killed probe left is swept; copies build warm
/// from one per-repository target directory; a SHA-256 repository works.
#[tokio::test]
async fn stale_copies_are_swept_builds_are_warm_and_sha256_repositories_work() {
    let warm = "case \"$CARGO_TARGET_DIR\" in */archon-probe-targets/*) test -f built.txt;; *) test -d .;; esac";
    let trees = trees_in(
        &[
            ("AC-M-001", warm, PROJECT),
            ("AC-M-002", "test -f feature.txt", REPO),
        ],
        &["init", "-q", "--object-format=sha256"],
    );
    assert_eq!(trees.base.len(), 64, "a SHA-256 repository");
    let copies = tempfile::tempdir().unwrap();
    let stale = copies.path().join("archon-probe-left-by-a-killed-run");
    std::fs::create_dir_all(&stale).unwrap();
    let old = std::process::Command::new("touch")
        .args(["-t", "202001010000"])
        .arg(&stale)
        .status()
        .unwrap();
    assert!(old.success());
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    assert_eq!(
        probe.baseline.as_ref().map(|b| b.commit.as_str()),
        Some(trees.base.as_str()),
        "the recorded SHA-256 base"
    );
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(
        findings.is_empty(),
        "{findings:?} {:?}",
        probe.take_diagnostics()
    );
    assert!(!stale.exists(), "the stale copy was swept");
    trees.assert_live_untouched(copies.path());
}

/// Fix 2, through the republish path (`freeze-acceptance --reauthor`): a
/// named check that failed its own assertion where it ran is held to that
/// verdict -- a repair that turns it green there goes back to its author.
#[tokio::test]
async fn the_republish_path_holds_a_repair_to_its_originals_failing_verdict() {
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    use crate::command::workflow_task_set::reauthor::{AuthorScope, ReauthorGate};
    use crate::command::workflow_task_set::republish::test_fixture::{
        NO_SEEDS, frozen_set_proven as frozen_set,
    };
    use crate::command::workflow_task_set::republish::{ReauthorRequest, reauthor_and_republish};
    let set = frozen_set(&[("AC-F-001", "test -f missing", true)]);
    std::fs::write(set.project.path().join("present"), "x").unwrap();
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| {
            command_entry(
                entry,
                if attempt == 1 {
                    "test -f present"
                } else {
                    "test -f missing && test -s missing"
                },
            )
        },
        |_, _| true,
    );
    let named = ["AC-F-001".to_string()].into_iter().collect();
    reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &set.probe,
                seeds: &NO_SEEDS,
            },
            trigger: "test",
        },
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd),
    )
    .await
    .expect("the repair that keeps the verdict publishes");
    assert_eq!(
        client.authored(),
        2,
        "the weakening repair cost its attempt"
    );
    let prompts = client.prompts.lock().unwrap();
    assert!(
        prompts[1].contains("failed its own assertion"),
        "{}",
        prompts[1]
    );
}
