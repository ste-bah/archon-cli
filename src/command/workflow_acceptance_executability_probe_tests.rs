//! A4/A5 on real trees: a repository in its own temporary directory, a
//! project outside it, a base commit before the feature and a HEAD after.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use archon_workflow::task_set_contract::{AcceptanceCheck, AcceptanceContract, TrustedCwd};

use super::{ExecutabilityProbe, FailedTree, HostProbe, Original};
use crate::command::workflow_task_set::republish::test_fixture::{FrozenSet, frozen_set};

pub(crate) struct Trees {
    pub(crate) set: FrozenSet,
    /// Holds the repository, apart from the project.
    pub(crate) outside: tempfile::TempDir,
    pub(crate) repo: PathBuf,
    /// The commit before the feature, and HEAD with it.
    pub(crate) base: String,
    pub(crate) head: String,
}

pub(crate) fn git(repo: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Checks (id, command, cwd), all judged accepted, in a project outside a
/// repository whose `repository.lock` records the base commit. The project
/// holds `data/state.txt` ("ready"); the repository tracks `src/state.txt`
/// ("ready") at base and adds `feature.txt` at HEAD.
pub(crate) fn trees(checks: &[(&str, &str, TrustedCwd)]) -> Trees {
    trees_in(checks, &["init", "-q"])
}

/// As [`trees`], the repository created with `init` (e.g. a SHA-256 one).
pub(crate) fn trees_in(checks: &[(&str, &str, TrustedCwd)], init: &[&str]) -> Trees {
    let set = frozen_set(
        &checks
            .iter()
            .map(|(id, command, _)| (*id, *command, true))
            .collect::<Vec<_>>(),
    );
    let outside = tempfile::tempdir().unwrap();
    let repo = outside.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, init);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "test"]);
    std::fs::write(repo.join("src/state.txt"), "ready").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("feature.txt"), "built").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "feature"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let record = archon_workflow::repository_record::RepositoryRecordV1 {
        schema_version: archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
        repository_root: repo
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap()
            .display()
            .to_string(),
        base_commit: base.clone(),
        decomposition_run_id: "fixture".into(),
        recorded_at: "2026-09-01T00:00:00Z".into(),
    };
    archon_workflow::repository_record::write_repository_record(&set.tasks, &record).unwrap();
    std::fs::create_dir_all(set.project.path().join("data")).unwrap();
    std::fs::write(set.project.path().join("data/state.txt"), "ready").unwrap();
    assert!(
        !set.project
            .path()
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap()
            .starts_with(repo.canonicalize().map(archon_shell::paths::plain).unwrap())
    );
    let trees = Trees {
        set,
        outside,
        repo,
        base,
        head,
    };
    trees.set_cwds(checks);
    trees
}

impl Trees {
    /// The frozen contract with each check's working directory as declared.
    pub(crate) fn contract(&self) -> AcceptanceContract {
        let mut contract = self.set.contract();
        let cwds: BTreeMap<String, TrustedCwd> =
            serde_json::from_str(&std::fs::read_to_string(self.cwds_path()).unwrap()).unwrap();
        for entry in &mut contract.acceptance {
            if let AcceptanceCheck::Command { cwd, .. } = &mut entry.check {
                *cwd = cwds[&entry.id];
            }
        }
        contract
    }

    fn cwds_path(&self) -> PathBuf {
        self.outside.path().join("cwds.json")
    }

    fn set_cwds(&self, checks: &[(&str, &str, TrustedCwd)]) {
        let cwds: BTreeMap<&str, TrustedCwd> =
            checks.iter().map(|(id, _, cwd)| (*id, *cwd)).collect();
        std::fs::write(self.cwds_path(), serde_json::to_vec(&cwds).unwrap()).unwrap();
    }

    pub(crate) fn ids(&self) -> BTreeSet<String> {
        self.contract()
            .acceptance
            .iter()
            .map(|entry| entry.id.clone())
            .collect()
    }

    /// No copy, marker or change is left in either live root.
    pub(crate) fn assert_live_untouched(&self, copies: &Path) {
        assert!(!self.set.project.path().join("executed-marker").exists());
        assert!(!self.repo.join("executed-marker").exists());
        assert_eq!(git(&self.repo, &["status", "--porcelain"]), "");
        assert_eq!(
            std::fs::read_to_string(self.set.project.path().join("data/state.txt")).unwrap(),
            "ready"
        );
        let left: Vec<_> = (std::fs::read_dir(copies).unwrap().flatten())
            .map(|entry| entry.file_name())
            .filter(|name| name != super::hermetic::WARM_TARGETS)
            .collect();
        assert!(left.is_empty(), "copies removed: {left:?}");
    }
}

/// Entries of a scratch parent other than its persistent build cache.
pub(crate) fn scratch_left(scratch: &Path) -> Vec<std::ffi::OsString> {
    (std::fs::read_dir(scratch).unwrap().flatten())
        .map(|entry| entry.file_name())
        .filter(|name| name != "build-cache")
        .collect()
}

const PROJECT: TrustedCwd = TrustedCwd::ProjectRoot;
const REPO: TrustedCwd = TrustedCwd::RepoRoot;

/// The operator's layout with no scratch policy: the freeze probe runs every
/// check in its own copy of the base tree and of the project's data, and
/// proves each can fail.
#[tokio::test]
async fn a_freeze_with_no_policy_proves_checks_can_fail_for_a_project_outside_the_repository() {
    let trees = trees(&[
        ("AC-P-001", "test -f built.txt", PROJECT),
        ("AC-P-002", "python3 -c 'import sys; sys.exit(0)'", PROJECT),
        ("AC-P-003", "grep -q ready data/state.txt", PROJECT),
        ("AC-P-004", "test -f feature.txt", REPO),
        (
            "AC-P-005",
            "touch executed-marker && test -f built.txt",
            PROJECT,
        ),
        ("AC-P-006", "grep -q ready src/state.txt", REPO),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    let baseline = probe.baseline.clone().expect("the task set's base commit");
    assert_eq!(
        baseline.commit, trees.base,
        "the recorded base, not HEAD {}",
        trees.head
    );
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let diagnostics = probe.take_diagnostics();
    assert_eq!(
        findings.keys().collect::<Vec<_>>(),
        vec!["AC-P-002"],
        "{findings:?} {diagnostics:?}"
    );
    let vacuous = &findings["AC-P-002"];
    assert!(
        vacuous.contains("passed on the pre-implementation tree")
            && vacuous.contains("it names none that exists there"),
        "{vacuous}"
    );
    for (id, input) in [
        ("AC-P-003", "data/state.txt"),
        ("AC-P-006", "src/state.txt"),
    ] {
        assert!(
            diagnostics
                .iter()
                .any(|d| d.contains(id) && d.contains("regression guard") && d.contains(input)),
            "{id}: {diagnostics:?}"
        );
    }
    trees.assert_live_untouched(copies.path());
}

/// With a scratch policy for a project outside the repository, the same
/// proof runs in the configured scratch observation, and a tracked file the
/// mutation moves aside is restored before the scratch audits itself.
#[tokio::test]
async fn a_freeze_with_a_scratch_policy_proves_checks_can_fail_in_scratch() {
    let trees = trees(&[
        ("AC-P-001", "test -f built.txt", PROJECT),
        ("AC-P-002", "python3 -c 'import sys; sys.exit(0)'", PROJECT),
        ("AC-P-003", "grep -q ready data/state.txt", PROJECT),
        ("AC-P-004", "test -f feature.txt", REPO),
        ("AC-P-006", "grep -q ready src/state.txt", REPO),
    ]);
    let scratch = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch.display().to_string(),
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    assert!(matches!(probe.site, super::Site::Scratch(_)));
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let diagnostics = probe.take_diagnostics();
    assert_eq!(
        findings.keys().collect::<Vec<_>>(),
        vec!["AC-P-002"],
        "{findings:?} {diagnostics:?}"
    );
    for id in ["AC-P-003", "AC-P-006"] {
        assert!(
            diagnostics
                .iter()
                .any(|d| d.contains(id) && d.contains("regression guard")),
            "{id}: {diagnostics:?}"
        );
    }
    assert!(
        scratch_left(&scratch).is_empty(),
        "scratch removed: {:?}",
        scratch_left(&scratch)
    );
    trees.assert_live_untouched(copies.path());
}

/// A5 on the round's own site: a repair must fail on the base commit, and
/// must not newly pass on the tree its original failed its own assertion on
/// -- unless that failure was the check's own defect.
#[tokio::test]
async fn a_repair_is_held_to_its_originals_verdict_only_where_that_verdict_was_real() {
    let passes_at_head = "test -f feature.txt";
    let trees = trees(&[
        ("AC-R-001", passes_at_head, REPO),
        ("AC-R-002", passes_at_head, REPO),
        ("AC-R-003", "test -f feature.txt && test -f built.txt", REPO),
    ]);
    let originals = BTreeMap::from([
        ("AC-R-001".to_string(), Original::Failed),
        ("AC-R-002".to_string(), Original::Defect),
        ("AC-R-003".to_string(), Original::Failed),
    ]);
    let copies = tempfile::tempdir().unwrap();
    // The round without a policy: its site is the live repository at HEAD.
    let probe = HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    )
    .with_baseline(super::Baseline {
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    })
    .with_failed_tree(FailedTree {
        commit: None,
        originals,
    })
    .with_copy_parent(copies.path().to_path_buf());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert_eq!(
        findings.keys().collect::<Vec<_>>(),
        vec!["AC-R-001"],
        "{findings:?} {:?}",
        probe.take_diagnostics()
    );
    assert!(
        findings["AC-R-001"].contains("failed its own assertion")
            && findings["AC-R-001"].contains("never turn a failing product green"),
        "{}",
        findings["AC-R-001"]
    );
    trees.assert_live_untouched(copies.path());
}

/// A5 on a tree the site is not on: the repair is run at that commit, in a
/// hermetic copy, and held to the original's verdict there.
#[tokio::test]
async fn a_repair_is_probed_on_the_failed_tree_when_the_site_is_elsewhere() {
    let trees = trees(&[
        ("AC-R-001", "grep -q ready src/state.txt", REPO),
        ("AC-R-002", "test -f feature.txt", REPO),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_failed_tree(FailedTree {
            commit: Some(trees.base.clone()),
            originals: BTreeMap::from([
                ("AC-R-001".to_string(), Original::Failed),
                ("AC-R-002".to_string(), Original::Failed),
            ]),
        })
        .with_copy_parent(copies.path().to_path_buf());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert_eq!(
        findings.keys().collect::<Vec<_>>(),
        vec!["AC-R-001"],
        "{findings:?} {:?}",
        probe.take_diagnostics()
    );
    trees.assert_live_untouched(copies.path());
}

/// A host placeholder (a criterion owed a check nobody authored) is never a
/// strength baseline: even a failing result of it is no verdict of its own.
#[test]
fn a_placeholder_original_is_never_a_verdict_to_hold_a_repair_to() {
    use crate::command::workflow_task_set::republish::test_fixture::criterion;
    let mut placeholder = criterion("AC-H-001", "false", false);
    placeholder.judgment.reason = super::PLACEHOLDER_REASON.into();
    let authored = criterion("AC-H-002", "test -f built.txt", true);
    assert!(super::is_placeholder(&placeholder));
    assert!(!super::is_placeholder(&authored));
    let mut refuted = criterion("AC-H-003", "test -f built.txt", false);
    refuted.judgment.reason = "the check misses a branch".into();
    assert!(
        !super::is_placeholder(&refuted),
        "an authored, refuted check is not one"
    );
    let mut contract = frozen_set(&[]).contract();
    contract.acceptance = vec![placeholder, authored];
    let failed = |id: &str| archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: id.into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        operational_error: None,
    };
    let results = [failed("AC-H-001"), failed("AC-H-002")];
    let originals = super::originals(&contract, &results);
    assert_eq!(originals["AC-H-001"], Original::Defect);
    assert_eq!(originals["AC-H-002"], Original::Failed);
}

/// (3) A probe run the host could not complete is repaired before it is
/// anyone's finding: a plain re-run on a fresh copy recovers it, and every
/// repair is recorded.
#[tokio::test]
async fn a_probe_run_the_host_could_not_complete_is_repaired_and_recorded() {
    let trees = trees(&[("AC-E-001", "test -f feature.txt", REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let round = |failures: usize| {
        HostProbe::at(
            trees.set.project.path().to_path_buf(),
            trees.repo.clone(),
            None,
        )
        .with_baseline(super::Baseline {
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        })
        .with_copy_parent(copies.path().to_path_buf())
        .with_injected_failures(failures)
    };
    let probe = round(1);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let diagnostics = probe.take_diagnostics();
    assert!(findings.is_empty(), "recovered by the re-run: {findings:?}");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("host-environment repair of the probe at")
                && d.contains("re-ran it on a freshly built copy")
                && d.contains("injected host failure")),
        "{diagnostics:?}"
    );
    // Fix 6: still unrunnable after every repair: the host's, never the
    // author's -- unproven, with the repair log. A copy has no build cache,
    // so one repair is all there is.
    let probe = round(5);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "never an author finding: {findings:?}");
    let unproven = probe.take_unproven();
    let why = &unproven["AC-E-001"];
    assert!(
        why.contains("could not be run on the pre-implementation tree")
            && why.contains(
                "after the host repaired its environment: re-ran it on a freshly built copy"
            ),
        "{why}"
    );
    assert_eq!(
        probe
            .injected_failures
            .load(std::sync::atomic::Ordering::SeqCst),
        3,
        "one run and one repaired re-run"
    );
    trees.assert_live_untouched(copies.path());
}

/// (3) With a scratch site that has a build cache, the second repair runs
/// without it.
#[tokio::test]
async fn a_scratch_probe_that_still_cannot_run_is_rerun_without_its_build_cache() {
    let trees = trees(&[("AC-E-001", "test -f built.txt", REPO)]);
    let scratch = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch.display().to_string(),
        ),
    )
    .unwrap();
    let cache = trees.outside.path().join("build-cache");
    let mut binding = crate::command::acceptance_scratch_policy::capture(
        trees.set.project.path(),
        &trees.set.tasks,
    )
    .unwrap()
    .expect("the configured scratch site");
    binding.policy.build_cache = Some(cache.clone());
    // The site run and its plain re-run fail; the baseline is the site's
    // own HEAD, so its verdict is the site's.
    let probe = HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        Some(binding),
    )
    .with_baseline(super::Baseline {
        commit: trees.head.clone(),
        repository: trees.repo.clone(),
    })
    .with_injected_failures(2);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let diagnostics = probe.take_diagnostics();
    assert!(findings.is_empty(), "{findings:?} {diagnostics:?}");
    let repairs: Vec<&String> = (diagnostics.iter())
        .filter(|d| d.contains("host-environment repair of the probe at"))
        .collect();
    assert_eq!(repairs.len(), 2, "{diagnostics:?}");
    assert!(
        repairs[1].contains("without the run's compiled-artifact cache"),
        "{repairs:?}"
    );
    assert!(!cache.exists(), "the cold re-run never used the cache");
    assert!(
        scratch_left(&scratch).is_empty(),
        "scratch removed: {:?}",
        scratch_left(&scratch)
    );
}
