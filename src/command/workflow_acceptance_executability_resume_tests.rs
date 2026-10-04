//! A freeze probe across attempts, on real trees (Issue 255): verdicts
//! saved and reused, the budget ending a freeze resumable, and the
//! per-check cap.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::time::{Duration, Instant};

use archon_workflow::task_set_contract::{AcceptanceCheck, AcceptanceContract, TrustedCwd};

use super::super::probe_tests::{Trees, trees};
use super::super::{ExecutabilityProbe, HostProbe};
use crate::command::workflow_freeze_budget::{
    FREEZE_INCOMPLETE_RESUMABLE, FreezeBudget, FreezeResume,
};

const REPO: TrustedCwd = TrustedCwd::RepoRoot;

/// A check that records each run in `runs` (outside every live root), then
/// passes only where the feature is built: at HEAD, never at the base.
fn counted(runs: &Path, id: &str, tail: &str) -> String {
    format!("echo run >> {}/{id}; {tail}", runs.display())
}

fn runs_of(runs: &Path, id: &str) -> usize {
    std::fs::read_to_string(runs.join(id)).map_or(0, |text| text.lines().count())
}

fn saving(budget: FreezeBudget) -> FreezeResume {
    FreezeResume::saving(budget, true)
}

/// A freeze probe in a new process: verdicts come only from disk.
fn freeze(trees: &Trees, copies: &Path, resume: &FreezeResume) -> HostProbe {
    HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.to_path_buf())
        .with_resume(resume)
        .without_process_memo()
}

fn with_command(mut contract: AcceptanceContract, id: &str, text: &str) -> AcceptanceContract {
    for entry in &mut contract.acceptance {
        if entry.id == id
            && let AcceptanceCheck::Command { command, .. } = &mut entry.check
        {
            *command = text.to_string();
        }
    }
    contract
}

#[tokio::test]
async fn saved_verdicts_are_reused_by_a_retry_and_never_after_their_inputs_change() {
    let runs = tempfile::tempdir().unwrap();
    let check = counted(runs.path(), "AC-R-001", "test -f feature.txt");
    let trees = trees(&[("AC-R-001", &check, REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let resume = saving(FreezeBudget::unlimited());

    let first = freeze(&trees, copies.path(), &resume);
    let findings = first.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(
        findings.is_empty(),
        "{findings:?} {:?}",
        first.take_diagnostics()
    );
    assert_eq!(
        runs_of(runs.path(), "AC-R-001"),
        2,
        "at HEAD and at the base"
    );

    let retry = freeze(&trees, copies.path(), &resume);
    let again = retry.script_defects(&trees.contract(), &trees.ids()).await;
    assert_eq!(again, findings);
    assert_eq!(runs_of(runs.path(), "AC-R-001"), 2, "nothing ran again");
    assert_eq!(
        retry.copies_made.load(SeqCst),
        0,
        "not even a copy was made"
    );
    assert!(retry.incomplete().is_none());

    // A changed check is a different input.
    let changed = with_command(
        trees.contract(),
        "AC-R-001",
        &counted(runs.path(), "AC-R-001", "test -f  feature.txt"),
    );
    freeze(&trees, copies.path(), &resume)
        .script_defects(&changed, &trees.ids())
        .await;
    assert_eq!(runs_of(runs.path(), "AC-R-001"), 4);

    // So is changed project data.
    std::fs::write(trees.set.project.path().join("data/extra.txt"), "new").unwrap();
    freeze(&trees, copies.path(), &resume)
        .script_defects(&trees.contract(), &trees.ids())
        .await;
    assert_eq!(runs_of(runs.path(), "AC-R-001"), 6);

    // An unsaving freeze (unstaged) neither reads nor writes them.
    let unsaved = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf())
        .without_process_memo();
    unsaved
        .script_defects(&trees.contract(), &trees.ids())
        .await;
    assert_eq!(runs_of(runs.path(), "AC-R-001"), 8);
}

/// A clock that stands still for its first `reads` reads, then jumps a
/// day: the budget is spent exactly when the freeze moves on to the base.
fn spent_after(reads: u64) -> crate::command::workflow_freeze_budget::Clock {
    let start = Instant::now();
    let count = Arc::new(AtomicU64::new(0));
    Arc::new(move || {
        let late = count.fetch_add(1, SeqCst) >= reads;
        start + Duration::from_secs(if late { 86_400 } else { 0 })
    })
}

#[tokio::test]
async fn a_spent_budget_ends_the_freeze_resumable_with_its_progress_saved() {
    let runs = tempfile::tempdir().unwrap();
    let one = counted(runs.path(), "AC-D-001", "test -f feature.txt");
    let two = counted(runs.path(), "AC-D-002", "test -s feature.txt");
    let trees = trees(&[("AC-D-001", &one, REPO), ("AC-D-002", &two, REPO)]);
    let copies = tempfile::tempdir().unwrap();
    // The budget's own start (one read), HEAD's copy (one) and its two
    // checks (one each) fit; the base's copy does not.
    let budget = FreezeBudget::within(7_800, spent_after(4));
    let first = freeze(&trees, copies.path(), &saving(budget));
    let findings = first.script_defects(&trees.contract(), &trees.ids()).await;
    let incomplete = first.incomplete().expect("the freeze ran out of budget");
    let text = incomplete.to_string();
    assert!(text.starts_with(FREEZE_INCOMPLETE_RESUMABLE), "{text}");
    assert!(text.contains("2 probe result(s) were saved"), "{text}");
    assert!(text.contains("AC-D-001, AC-D-002"), "{text}");
    assert!(
        findings.is_empty(),
        "nothing of a partial probe is final: {findings:?}"
    );
    assert_eq!(runs_of(runs.path(), "AC-D-001"), 1, "HEAD only");
    assert_eq!(runs_of(runs.path(), "AC-D-002"), 1);

    let retry = freeze(&trees, copies.path(), &saving(FreezeBudget::unlimited()));
    let findings = retry.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "{findings:?}");
    assert!(retry.incomplete().is_none());
    assert!(retry.take_unproven().is_empty());
    assert_eq!(
        runs_of(runs.path(), "AC-D-001"),
        2,
        "HEAD reused; the base run"
    );
    assert_eq!(runs_of(runs.path(), "AC-D-002"), 2);
    assert_eq!(retry.copies_made.load(SeqCst), 1, "only the base's copy");
}

#[tokio::test]
async fn a_check_past_the_cap_is_unproven_timed_out_and_never_rerun() {
    let runs = tempfile::tempdir().unwrap();
    let slow = counted(runs.path(), "AC-T-001", "sleep 5; test -f feature.txt");
    let trees = trees(&[("AC-T-001", &slow, REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let probe = freeze(&trees, copies.path(), &saving(FreezeBudget::unlimited())).with_check_cap(1);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "never the author's: {findings:?}");
    let unproven = probe.take_unproven();
    assert!(
        unproven
            .get("AC-T-001")
            .is_some_and(|why| why.contains("unproven (timed out)")),
        "{unproven:?}"
    );
    assert!(
        probe.incomplete().is_none(),
        "a timeout is not a spent budget"
    );
    assert_eq!(
        runs_of(runs.path(), "AC-T-001"),
        2,
        "once per tree, no repair re-runs"
    );
    let retry = freeze(&trees, copies.path(), &saving(FreezeBudget::unlimited()));
    retry
        .with_check_cap(1)
        .script_defects(&trees.contract(), &trees.ids())
        .await;
    assert_eq!(
        runs_of(runs.path(), "AC-T-001"),
        4,
        "a timeout is never saved"
    );
}

/// The scratch site of a configured `[workflow.acceptance_execution]`, as
/// the live freeze probes: verdicts are saved under its build cache, apart
/// from every live root, and the budget stops it between observations.
#[tokio::test]
async fn a_scratch_freeze_saves_under_its_build_cache_and_resumes() {
    let runs = tempfile::tempdir().unwrap();
    let one = counted(runs.path(), "AC-S-001", "test -f feature.txt");
    let two = counted(runs.path(), "AC-S-002", "test -s feature.txt");
    let trees = trees(&[("AC-S-001", &one, REPO), ("AC-S-002", &two, REPO)]);
    let scratch = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.canonicalize().unwrap(),
            scratch
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let saved = || -> usize {
        let caches = std::fs::read_dir(scratch.join("build-cache"))
            .unwrap()
            .flatten();
        (caches.filter_map(|cache| std::fs::read_dir(cache.path().join("probe-results")).ok()))
            .map(|entries| entries.count())
            .sum()
    };
    let budget = FreezeBudget::within(7_800, spent_after(4));
    let first = freeze(&trees, copies.path(), &saving(budget));
    first.script_defects(&trees.contract(), &trees.ids()).await;
    let incomplete = first.incomplete().expect("the base was never observed");
    assert!(
        incomplete.to_string().contains("AC-S-001, AC-S-002"),
        "{incomplete}"
    );
    assert_eq!(saved(), 2, "HEAD's two verdicts, under the build cache");
    assert_eq!(runs_of(runs.path(), "AC-S-001"), 1);

    let retry = freeze(&trees, copies.path(), &saving(FreezeBudget::unlimited()));
    let findings = retry.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(
        findings.is_empty(),
        "{findings:?} {:?}",
        retry.take_diagnostics()
    );
    assert!(retry.incomplete().is_none());
    assert!(retry.take_unproven().is_empty());
    assert_eq!(
        runs_of(runs.path(), "AC-S-001"),
        2,
        "HEAD reused, the base run"
    );
    assert_eq!(runs_of(runs.path(), "AC-S-002"), 2);
    assert_eq!(saved(), 4);
    trees.assert_live_untouched(copies.path());
}

/// A check that already passes before any implementation is proven by an
/// input mutation; a retry reuses that verdict as well (its nonce is kept).
#[tokio::test]
async fn a_saved_mutation_verdict_is_reused_by_a_retry() {
    let runs = tempfile::tempdir().unwrap();
    let guard = counted(runs.path(), "AC-M-001", "grep -q ready src/state.txt");
    let trees = trees(&[("AC-M-001", &guard, REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let resume = saving(FreezeBudget::unlimited());
    let first = freeze(&trees, copies.path(), &resume);
    let findings = first.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "a regression guard: {findings:?}");
    assert!(
        first
            .take_diagnostics()
            .iter()
            .any(|d| d.contains("regression guard")),
        "it was proven by its mutation"
    );
    assert_eq!(runs_of(runs.path(), "AC-M-001"), 3, "HEAD, base, mutation");
    let retry = freeze(&trees, copies.path(), &resume);
    let again = retry.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(again.is_empty(), "{again:?}");
    assert!(
        retry
            .take_diagnostics()
            .iter()
            .any(|d| d.contains("regression guard")),
        "the same proof, from the saved verdict"
    );
    assert_eq!(runs_of(runs.path(), "AC-M-001"), 3, "nothing ran again");
    assert_eq!(retry.copies_made.load(SeqCst), 0);
}

#[test]
fn a_crash_before_validation_leaves_no_reusable_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let store = super::ResultStore::new(
        dir.path().to_path_buf(),
        crate::command::workflow_task_set::passability::evidence::Redactor::from_vars(
            Vec::new(),
            &[],
        ),
    );
    let key = "a".repeat(64);
    let result: archon_workflow::acceptance_scratch::CheckResult =
        serde_json::from_value(serde_json::json!({
            "acceptance_id":"AC-C-001", "passed":true, "exit_code":0,
            "stdout":[], "stderr":[], "operational_error":null
        }))
        .unwrap();
    assert!(store.save(&key, &result));
    assert!(store.load(&key).is_none());
    assert!(
        !store
            .path(&key)
            .unwrap()
            .with_extension("provisional")
            .exists()
    );
}

/// Each retry appends to the task root's `.decompose.log`; that must not
/// change the verdict key, or no retry could ever reuse a saved verdict.
#[tokio::test]
async fn a_retry_reuses_verdicts_after_the_decompose_log_grows() {
    let runs = tempfile::tempdir().unwrap();
    let check = counted(runs.path(), "AC-L-001", "test -f feature.txt");
    let trees = trees(&[("AC-L-001", &check, REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let resume = saving(FreezeBudget::unlimited());
    let log = Path::new(&trees.set.tasks).join(".decompose.log");

    std::fs::write(&log, "attempt 1\n").unwrap();
    freeze(&trees, copies.path(), &resume)
        .script_defects(&trees.contract(), &trees.ids())
        .await;
    assert_eq!(runs_of(runs.path(), "AC-L-001"), 2);

    // What the executor writes before a retry.
    std::fs::write(&log, "attempt 1\nretry\n").unwrap();
    let retry = freeze(&trees, copies.path(), &resume);
    retry.script_defects(&trees.contract(), &trees.ids()).await;
    assert_eq!(runs_of(runs.path(), "AC-L-001"), 2, "nothing ran again");

    // The exclusion is narrow: other task-root content still keys.
    std::fs::write(Path::new(&trees.set.tasks).join("note.md"), "new").unwrap();
    freeze(&trees, copies.path(), &resume)
        .script_defects(&trees.contract(), &trees.ids())
        .await;
    assert_eq!(runs_of(runs.path(), "AC-L-001"), 4);
}

/// Issue 277: a saved verdict never holds a credential its check printed in
/// clear. The saved output is redacted and owner-only, and the verdict a
/// retry reads back is the one the first attempt reached.
#[tokio::test]
async fn saved_verdicts_hold_no_credential_their_check_printed() {
    const CANARY: &str = "sk-ant-277-probe-canary-9a1d";
    crate::command::workflow_task_set::passability::test_secret("SERVICE_TOKEN", CANARY);
    // The check text spells the value in two halves, so only the check's
    // output can carry it whole.
    let check = "printf 'token=sk-ant-%s\\n' 277-probe-canary-9a1d; \
                 printf 'auth sk-ant-%s\\n' 277-probe-canary-9a1d >&2; test -f feature.txt";
    let trees = trees(&[("AC-K-001", check, REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let resume = saving(FreezeBudget::unlimited());
    let first = freeze(&trees, copies.path(), &resume);
    let findings = first.script_defects(&trees.contract(), &trees.ids()).await;
    let results = trees
        .set
        .project
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        .join("probe-results");
    let saved: Vec<_> = std::fs::read_dir(&results)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    assert_eq!(saved.len(), 2, "HEAD and the base were saved");
    for path in &saved {
        // The output streams are saved as byte arrays, so a plain text
        // search would never see the value: decode them first.
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for stream in ["stdout", "stderr"] {
            let bytes: Vec<u8> = serde_json::from_value(saved["result"][stream].clone()).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                !text.contains(CANARY),
                "{} {stream}: {text}",
                path.display()
            );
            assert!(
                text.contains("REDACTED"),
                "{} {stream}: {text}",
                path.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{}", path.display());
        }
    }
    let retry = freeze(&trees, copies.path(), &resume);
    let again = retry.script_defects(&trees.contract(), &trees.ids()).await;
    assert_eq!(again, findings, "the saved verdict is the one reached");
    assert_eq!(retry.copies_made.load(SeqCst), 0, "read back, not re-run");
}
