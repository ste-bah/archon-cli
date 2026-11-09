use super::*;
use archon_workflow::base_snapshot::{BaseSnapshot, snapshot_deliverables};
use archon_workflow::baseline_results::{BaselineOutcome, BaselineResults};
use archon_workflow::repository_record::{RepositoryRecordV1, RepositoryTree};
use archon_workflow::task_skeleton::TaskSkeleton;
use archon_workflow::v2::requirement_classes::classify;
pub(super) fn git(root: &Path, args: &[&str]) -> String {
    let out = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
pub(super) struct Fixture {
    pub(super) source: tempfile::TempDir,
    pub(super) parent: tempfile::TempDir,
    pub(super) contract: AcceptanceContract,
    pub(super) lock: AcceptanceLock,
    pub(super) classes: RequirementClassification,
    pub(super) mutants: MutantSet,
}
impl Fixture {
    pub(super) fn new(command: &str) -> Self {
        let source = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        git(source.path(), &["init", "-q"]);
        git(source.path(), &["config", "user.name", "test"]);
        git(
            source.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(source.path(), &["config", "core.autocrlf", "false"]);
        for (path, bytes) in [
            ("value", "good\n"),
            ("other", "base\n"),
            (".gitignore", "ignored\n"),
        ] {
            std::fs::write(source.path().join(path), bytes).unwrap();
        }
        git(source.path(), &["add", "."]);
        git(source.path(), &["commit", "-qm", "base"]);
        let base = git(source.path(), &["rev-parse", "HEAD"]);
        let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
            "schema_version":1,"prd":{"path":"input","digest":"anchor"},"gap_policy":{},"supplementary":[],
            "acceptance":[{"id":"check","criterion":"item","check":{"kind":"command","command":command,"cwd":"repo_root"},"covers":["item"],"judgment":{"verdict":"accepted","counterexample":"fixture admission only","reason":"fixture","host_call_id":"test"}}]
        })).unwrap();
        let skeleton: TaskSkeleton = serde_json::from_value(serde_json::json!({
            "schema_version":1,"acceptance_digest":"anchor","tasks":[{"task_id":"task","file_name":"task","implements":["item"],"deliverable_contracts":[{"kind":"output","artifact_path":"value","min_instances":1,"implements":["item"]}]}]
        })).unwrap();
        let mut lock: AcceptanceLock = serde_json::from_slice(include_bytes!(
            "../../crates/archon-workflow/tests/fixtures/classification/acceptance.lock"
        ))
        .unwrap();
        lock.baseline_commit = Some(base.clone());
        lock.digest = content_digest(b"acceptance");
        lock.baseline_results = BaselineResults::observed(
            Some(base.clone()),
            &[("check".into(), BaselineOutcome::Passed)].into(),
            &contract,
        )
        .outcomes;
        let tree = RepositoryTree::load(&RepositoryRecordV1 {
            schema_version: 1,
            repository_root: source.path().display().to_string(),
            base_commit: base.clone(),
            decomposition_run_id: "test".into(),
            recorded_at: "test".into(),
        })
        .unwrap();
        let snapshot = snapshot_deliverables(&tree, source.path(), ["value", "absent"]).unwrap();
        let classes = RequirementClassification {
            schema_version: 2,
            baseline_commit: Some(base.clone()),
            acceptance_digest: lock.digest.clone(),
            baseline_results_digest: results_digest(&lock.baseline_results),
            error: None,
            entries: classify(&skeleton, &contract, &lock.baseline_results, |_| {
                (true, true)
            }),
            base_snapshot: Some(snapshot),
        };
        let mut mutants: MutantSet =
            serde_json::from_str(include_str!("../../tests/fixtures/breakage/mutants.json"))
                .unwrap();
        mutants.base_commit = base;
        Self {
            source,
            parent,
            contract,
            lock,
            classes,
            mutants,
        }
    }
    #[cfg(unix)]
    pub(super) fn probe(&self, persist: bool) -> HostProbe {
        let mut probe = HostProbe::new(
            self.source.path().into(),
            self.source.path().into(),
            Site::Hermetic,
        )
        .with_baseline(Baseline::head_of(self.source.path()).unwrap());
        probe.copy_parent = self.parent.path().into();
        probe.resume.persist = persist;
        probe
    }
    #[cfg(unix)]
    pub(super) async fn prove(&self, probe: &HostProbe, budget: u64) -> BreakageReport {
        prove_breakage(
            probe,
            BreakageRequest {
                contract: &self.contract,
                chain_digest: &self.lock.digest,
                lock: &self.lock,
                classification: &self.classes,
                mutants: &self.mutants,
                max_runs: budget,
                min_deadline_secs: 1,
            },
        )
        .await
    }
    pub(super) fn snapshot(&self) -> &BaseSnapshot {
        self.classes.base_snapshot.as_ref().unwrap()
    }
    pub(super) fn copy(&self) -> BaseCopy {
        materialise_base_copy(
            self.parent.path(),
            self.source.path(),
            self.source.path(),
            None,
            self.snapshot(),
            self.lock.baseline_commit.as_deref().unwrap(),
            &self.parent.path().join("output"),
            None,
        )
        .unwrap()
    }
}
#[cfg(unix)]
#[tokio::test]
async fn kills_cache_and_zero_budget_reuse() {
    let f = Fixture::new("test \"$(cat value)\" = good");
    let p = f.probe(true);
    let first = f.prove(&p, 10).await;
    assert_eq!(first.proofs.summary.as_ref().unwrap().runs_executed, 4);
    assert_eq!(
        first.proofs.summary.as_ref().unwrap().entries[0].state,
        ProofState::BehaviourKill
    );
    assert!(first.proofs.runs.values().all(|r| r.counted));
    for r in first.proofs.runs.values() {
        assert!(
            first
                .store
                .as_ref()
                .unwrap()
                .join(&r.evidence.patch)
                .is_file()
        );
        assert!(
            first
                .store
                .as_ref()
                .unwrap()
                .join(r.evidence.output.as_ref().unwrap())
                .is_file()
        );
    }
    let second = f.prove(&p, 0).await;
    let s = second.proofs.summary.unwrap();
    assert_eq!((s.runs_executed, s.runs_reused), (0, 2));
    assert_eq!(s.entries[0].state, ProofState::BehaviourKill);
    assert_eq!(
        std::fs::read(f.source.path().join("value")).unwrap(),
        b"good\n"
    );
}
#[cfg(unix)]
#[tokio::test]
async fn budget_mismatch_survival_and_body_gut() {
    for budget in [0, 1, 3] {
        let mut f = Fixture::new("test \"$(cat value)\" = good");
        for value in ["nope", "next"] {
            let mut mutant = f.mutants.mutants[0].clone();
            mutant.patch = mutant.patch.replace("evil", value);
            f.mutants.mutants.push(mutant);
        }
        let p = f.probe(false);
        let r = f.prove(&p, budget).await;
        assert_eq!(r.proofs.runs.len(), 4);
        assert_eq!(
            r.proofs
                .runs
                .values()
                .filter(|r| matches!(r.outcome, RunOutcome::BudgetExhausted))
                .count(),
            if budget < 2 { 4 } else { 3 }
        );
        assert!(r.proofs.summary.as_ref().unwrap().runs_executed <= budget);
        if budget < 2 {
            assert_eq!(
                r.proofs.summary.unwrap().entries[0].state,
                ProofState::BudgetExhausted
            );
        }
        assert!(r.store.is_none());
    }
    let mut f = Fixture::new("test \"$(cat value)\" = good");
    f.mutants.mutants.remove(0);
    assert_eq!(
        f.prove(&f.probe(false), 10)
            .await
            .proofs
            .summary
            .unwrap()
            .entries[0]
            .state,
        ProofState::BodyGutKillOnly
    );
    let f = Fixture::new("test -f ignored");
    std::fs::write(f.source.path().join("ignored"), b"live").unwrap();
    let r = f.prove(&f.probe(false), 10).await;
    assert!(
        r.proofs
            .runs
            .values()
            .all(|r| r.not_counted == Some(NotCounted::ControlMismatch))
    );
    let summary = r.proofs.summary.unwrap();
    assert_eq!(summary.runs_executed, 1);
    assert_eq!(summary.entries[0].state, ProofState::Unverified);
    let f = Fixture::new("test -f value");
    assert_eq!(
        f.prove(&f.probe(false), 10)
            .await
            .proofs
            .summary
            .unwrap()
            .entries[0]
            .state,
        ProofState::SurvivorsOnly
    );
}
#[test]
fn patch_round_trip_restore_and_inventory() {
    for (blob, replacement) in [
        (b"good\n".as_slice(), b"evil\n".as_slice()),
        (b"good\r\n", b"evil\r\n"),
        (b"aaaa\n", b"bbbb\n"),
    ] {
        let f = Fixture::new("true");
        std::fs::write(f.source.path().join("value"), blob).unwrap();
        git(f.source.path(), &["add", "value"]);
        git(
            f.source.path(),
            &["commit", "--allow-empty", "-qm", "bytes"],
        );
        let base = git(f.source.path(), &["rev-parse", "HEAD"]);
        let tree = RepositoryTree::load(&RepositoryRecordV1 {
            schema_version: 1,
            repository_root: f.source.path().display().to_string(),
            base_commit: base.clone(),
            decomposition_run_id: "test".into(),
            recorded_at: "test".into(),
        })
        .unwrap();
        let snapshot = snapshot_deliverables(&tree, f.source.path(), ["value"]).unwrap();
        std::fs::write(f.source.path().join("value"), replacement).unwrap();
        let diff = archon_shell::spawn::command("git")
            .arg("-C")
            .arg(f.source.path())
            .args(["diff", "--no-ext-diff"])
            .output()
            .unwrap()
            .stdout;
        let c = materialise_base_copy(
            f.parent.path(),
            f.source.path(),
            f.source.path(),
            None,
            &snapshot,
            &base,
            &f.parent.path().join("output"),
            None,
        )
        .unwrap();
        std::fs::write(c.repository().join("keep"), b"keep").unwrap();
        let before = patch::inventory(c.copy()).unwrap();
        let file = f.parent.path().join("change.patch");
        let old = std::fs::metadata(c.repository().join("value"))
            .unwrap()
            .modified()
            .unwrap();
        let touched = patch::apply(c.copy(), &file, &diff).unwrap();
        let applied = std::fs::metadata(c.repository().join("value"))
            .unwrap()
            .modified()
            .unwrap();
        assert!(applied > old);
        std::fs::write(c.repository().join("new"), b"new").unwrap();
        std::fs::write(c.repository().join("other"), b"dirty").unwrap();
        assert_eq!(
            patch::restore(c.copy(), &file, &touched, &before).unwrap(),
            1
        );
        assert_eq!(std::fs::read(c.repository().join("value")).unwrap(), blob);
        assert!(
            std::fs::metadata(c.repository().join("value"))
                .unwrap()
                .modified()
                .unwrap()
                > applied
        );
        assert!(c.repository().join("keep").is_file());
        assert!(!c.repository().join("new").exists());
    }
}
#[cfg(unix)]
#[path = "workflow_acceptance_executability_breakage_engine_tests.rs"]
mod engine_cases;
#[cfg(unix)]
#[path = "workflow_acceptance_executability_breakage_io_tests.rs"]
mod io_cases;
#[path = "workflow_acceptance_executability_breakage_patch_tests.rs"]
mod patch_cases;

#[path = "workflow_acceptance_executability_breakage_resolution_tests.rs"]
mod resolution_cases;
