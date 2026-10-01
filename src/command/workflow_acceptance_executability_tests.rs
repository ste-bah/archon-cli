//! The executability gate inside the bounded re-author loop.

use std::collections::BTreeSet;
use std::path::Path;

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceCheck, AcceptanceContract, AcceptancePin, JudgeDecision,
    TASK_SKELETON_FILE,
};
use archon_workflow::task_skeleton::validate_full_chain;

use super::HostProbe;
use crate::command::workflow_task_set::acceptance_pin_path;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::reauthor::{
    AuthorScope, REAUTHOR_ATTEMPTS, ReauthorGate, reauthor,
};
use crate::command::workflow_task_set::republish::test_fixture::{
    FrozenSet, NO_SEEDS, assert_only_named_entries_changed, assert_skeleton_only_rebound,
    frozen_set,
};
use crate::command::workflow_task_set::republish::{ReauthorRequest, reauthor_and_republish};

/// Calls a helper it defines with one argument too few: never asserts.
pub(crate) const CRASHING: &str = "python3 - <<'PY'\nimport os\ndef lane(path, d):\n    assert os.path.exists(path), 'deliverable missing: ' + path\nlane('present')\nPY\n";
/// The same assertion, runnable: fails only when the deliverable is missing.
pub(crate) const FIXED: &str = "python3 - <<'PY'\nimport os\ndef lane(path):\n    assert os.path.exists(path), 'deliverable missing: ' + path\nlane('present')\nPY\n";
const CRASH_SIGNAL: &str = "lane() missing 1 required positional argument: 'd'";

fn ids(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|id| id.to_string()).collect()
}

fn scope(set: &FrozenSet) -> AuthorScope {
    AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd)
}

fn command_of(set_contract: &archon_workflow::task_set_contract::AcceptanceContract) -> String {
    match &set_contract.acceptance[0].check {
        AcceptanceCheck::Command { command, .. } => command.clone(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_judged_accepted_check_that_crashes_in_its_own_code_goes_back_to_its_author() {
    let set = frozen_set(&[("AC-F-001", "test -f missing", false)]);
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, if attempt == 1 { CRASHING } else { FIXED }),
        |_, _| true,
    );
    let after = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect("the runnable re-author is accepted");
    assert_eq!(client.authored(), 2, "the crashing reply cost its attempt");
    assert_eq!(command_of(&after), FIXED);
    assert_eq!(
        after.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(
        client.judged_ids.lock().unwrap().len(),
        2,
        "the repair is judged again"
    );
    let prompts = client.prompts.lock().unwrap();
    assert!(
        prompts[1].contains(CRASH_SIGNAL) && prompts[1].contains("crashed in its own python code"),
        "the author is shown the real crash: {}",
        prompts[1]
    );
    assert!(prompts[1].contains("keep every assertion"));
}

#[tokio::test]
async fn a_check_failing_on_its_own_assertion_is_publishable_as_authored() {
    // No `present` file: the product is genuinely failing the criterion.
    let set = frozen_set(&[("AC-F-001", "test -f missing", false)]);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, FIXED), |_, _| true);
    let after = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect("an assertion failure is not a script defect");
    assert_eq!(client.authored(), 1);
    assert_eq!(command_of(&after), FIXED);
}

#[tokio::test]
async fn a_check_that_keeps_crashing_is_never_accepted() {
    let set = frozen_set(&[("AC-F-001", "test -f missing", false)]);
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("{CRASHING}# attempt {attempt}\n")),
        |_, _| true,
    );
    let error = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect_err("a crashing check is never accepted")
    .to_string();
    assert_eq!(client.authored(), REAUTHOR_ATTEMPTS);
    assert!(error.contains(CRASH_SIGNAL), "{error}");
}

#[tokio::test]
async fn a_named_accepted_check_that_crashes_is_shown_its_own_crash_first() {
    let set = frozen_set(&[("AC-F-001", CRASHING, true)]);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, FIXED), |_, _| true);
    let after = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect("repaired");
    assert_eq!(command_of(&after), FIXED);
    let prompts = client.prompts.lock().unwrap();
    assert!(prompts[0].contains(CRASH_SIGNAL), "{}", prompts[0]);
}

/// The dry run of `--reauthor` on a check that crashes in its own code: the
/// frozen check is probed (its real crash seeds the author), the scripted
/// author first returns that same crashing command (with a trailing comment,
/// so the repeat rule cannot pre-empt the probe) and then `from` replaced by
/// `to`. The gate must reject the first from its real crash output and
/// publish the second. The probe runs in `project` only.
async fn crash_dry_run(project: &Path, tasks: &Path, prd: &Path, spec: [&str; 3]) {
    let [check, from, to] = spec;
    let contract_path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let before = std::fs::read(&contract_path).unwrap();
    let skeleton_before = std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap();
    let contract: AcceptanceContract = serde_json::from_slice(&before).unwrap();
    let frozen = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .find(|entry| entry.id == check)
        .expect("the named check is frozen");
    let AcceptanceCheck::Command { command, .. } = &frozen.check else {
        panic!("{check} is not a command check");
    };
    let crashing = format!("{command}\n# dry run: the frozen crashing command\n");
    let fixed = command.replace(from, to);
    assert_ne!(&fixed, command, "the fix must change the check");
    let provider = contract.acceptance[0].judgment.sampling.as_ref().unwrap()["provider"]
        .as_str()
        .unwrap()
        .to_string();
    let replies = [crashing, fixed.clone()];
    let client = ScriptedAuthorJudge::new(
        move |entry, attempt| command_entry(entry, &replies[(attempt - 1).min(1)]),
        |_, _| true,
    )
    .with_provider(&provider);
    let probe = HostProbe::at(project.to_path_buf(), project.to_path_buf(), None);
    let named = ids(&[check]);
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: project,
            tasks_root: tasks,
            prd_path: prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &probe,
                seeds: &NO_SEEDS,
            },
            trigger: "test",
        },
        &AuthorScope::for_task_set(project, tasks, prd),
    )
    .await
    .expect("the runnable re-author publishes");
    let prompts = client.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2, "the crashing reply was rejected once");
    for (attempt, prompt) in prompts.iter().enumerate() {
        // Attempt 1 sees the frozen check's crash; attempt 2 also the crash
        // of the reply the gate rejected.
        let crashes = prompt
            .lines()
            .filter(|line| line.starts_with("- check '") && line.contains("crashed in its own"))
            .count();
        assert_eq!(crashes, attempt + 1, "attempt {}: {prompt}", attempt + 1);
        let findings = prompt
            .split_once("Findings to fix, oldest first:")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        eprintln!("dry run: attempt {} was shown:{findings}\n", attempt + 1);
    }
    let after = std::fs::read(&contract_path).unwrap();
    assert_only_named_entries_changed(&before, &after, &named);
    assert_skeleton_only_rebound(
        &skeleton_before,
        &std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap(),
    );
    let published: AcceptanceContract = serde_json::from_slice(&after).unwrap();
    let entry = (published.acceptance.iter())
        .chain(&published.supplementary)
        .find(|entry| entry.id == check)
        .unwrap();
    assert!(matches!(&entry.check, AcceptanceCheck::Command { command, .. } if *command == fixed));
    let pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(project, tasks)).unwrap())
            .unwrap();
    validate_full_chain(tasks, &pin).expect("the republished chain verifies");
    eprintln!(
        "dry run: {check} crash rejected, fixed check published; event={} diagnostics={:?}",
        result.freeze_event_id, result.diagnostics
    );
}

/// Against a COPY of a real frozen task directory when
/// `ARCHON_CRASH_REAUTHOR_DRY_RUN=<project>|<tasks>|<prd>|<check id>|<from>|<to>`
/// names one under a temporary directory; against a synthetic set otherwise.
#[tokio::test]
async fn crash_reauthor_dry_run_against_a_copied_task_directory() {
    let Ok(spec) = std::env::var("ARCHON_CRASH_REAUTHOR_DRY_RUN") else {
        let set = frozen_set(&[("AC-F-001", CRASHING, true)]);
        let spec = ["AC-F-001", "def lane(path, d):", "def lane(path):"];
        crash_dry_run(set.project.path(), &set.tasks, &set.prd, spec).await;
        return;
    };
    let parts = spec.split('|').collect::<Vec<_>>();
    let [project, tasks, prd, check, from, to] = parts.as_slice() else {
        panic!("ARCHON_CRASH_REAUTHOR_DRY_RUN must be <project>|<tasks>|<prd>|<check>|<from>|<to>");
    };
    let temp = std::env::temp_dir().canonicalize().unwrap();
    for path in [project, tasks, prd] {
        let canonical = Path::new(path).canonicalize().expect("dry-run path exists");
        assert!(
            canonical.starts_with(&temp) || canonical.starts_with("/private/tmp"),
            "refusing a dry run outside a temporary directory: {}",
            canonical.display()
        );
    }
    crash_dry_run(
        Path::new(project),
        Path::new(tasks),
        Path::new(prd),
        [check, from, to],
    )
    .await;
}

/// A git repository and a `[workflow.acceptance_execution]` policy for
/// `project`, as an operator configures one. Returns the directory holding
/// the repository and the scratch parent.
pub(crate) fn configure_scratch(project: &Path) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "test"]);
    std::fs::write(repo.join("input"), "source").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"combined\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=30\noutput_bytes=8192\nscratch_bytes=16777216\n",
            repo.display().to_string(),
            temp.path().join("scratch").display().to_string(),
        ),
    )
    .unwrap();
    temp
}

/// With `[workflow.acceptance_execution]` configured, the freeze-time probe
/// runs the check in the same hermetic scratch observation the acceptance
/// stage uses, and leaves nothing behind.
#[tokio::test]
async fn the_probe_runs_checks_in_the_configured_scratch_site() {
    use super::ExecutabilityProbe;
    let crashing = frozen_set(&[("AC-F-001", CRASHING, true)]);
    let scratch = configure_scratch(crashing.project.path());
    let probe = HostProbe::for_task_set(crashing.project.path(), &crashing.tasks);
    let found = probe
        .script_defects(&crashing.contract(), &ids(&["AC-F-001"]))
        .await;
    assert!(
        found
            .get("AC-F-001")
            .is_some_and(|f| f.contains(CRASH_SIGNAL)),
        "{found:?} {:?}",
        probe.take_diagnostics()
    );
    let fixed = frozen_set(&[("AC-F-001", FIXED, true)]).contract();
    assert!(
        probe
            .script_defects(&fixed, &ids(&["AC-F-001"]))
            .await
            .is_empty()
    );
    let diagnostics: Vec<String> = (probe.take_diagnostics().into_iter())
        .filter(|d| !d.contains("builds warm from the scratch build cache"))
        .collect();
    assert!(
        diagnostics.is_empty(),
        "both probes ran in scratch: {diagnostics:?}"
    );
    // Only the persistent warm build cache stays.
    let left: Vec<_> = (std::fs::read_dir(scratch.path().join("scratch"))
        .unwrap()
        .flatten())
    .map(|entry| entry.file_name())
    .filter(|name| name != "build-cache")
    .collect();
    assert!(
        left.is_empty(),
        "the scratch copy and the probe's evidence were removed: {left:?}"
    );
}

/// With no scratch policy a freeze never executes an authored check in a
/// live root: it runs it in its own hermetic copy of the tree, and gives a
/// verdict (the old behaviour ran nothing and proved nothing).
#[tokio::test]
async fn a_freeze_without_a_scratch_policy_executes_only_in_a_hermetic_copy() {
    use super::ExecutabilityProbe;
    let set = frozen_set(&[("AC-F-001", "touch executed-marker && false", true)]);
    let copies = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(set.project.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "test"]);
    git(&["commit", "-q", "--allow-empty", "-m", "base"]);
    let probe = HostProbe::for_task_set(set.project.path(), &set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    let findings = probe
        .script_defects(&set.contract(), &ids(&["AC-F-001"]))
        .await;
    let diagnostics = probe.take_diagnostics();
    assert!(
        findings.is_empty(),
        "it fails before any implementation: {findings:?}"
    );
    assert!(diagnostics.is_empty(), "it ran: {diagnostics:?}");
    assert!(!set.project.path().join("executed-marker").exists());
    assert_eq!(std::fs::read_dir(copies.path()).unwrap().count(), 0);
}
