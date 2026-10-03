// Issue 255: a caller bounds each check and hears each result early,
// through a real scratch observation (Unix process groups, as the other
// acceptance_scratch targets).
#![cfg(unix)]

use archon_workflow::acceptance_scratch::{
    CHECK_DEFERRED, CHECK_TIMED_OUT, CheckAllowance, CheckHook, CheckResult, ObserveHooks,
    ScratchPolicy, observe_commands_hooked,
};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
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

/// A repository, a project and one accepted check per command.
fn fixture(
    commands: &[&str],
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
        timeout_secs: 30,
        output_bytes: 2048,
        scratch_bytes: 16 * 1024 * 1024,
        build_cache: None,
    };
    let entries: Vec<_> = (commands.iter().enumerate())
        .map(|(n, command)| serde_json::json!({"id":format!("AC-X-00{n}"),"criterion":"output correct","check":{"kind":"command","command":command,"cwd":"project_root"},"judgment":{"verdict":"accepted","counterexample":"incorrect output","reason":"checks output","host_call_id":"j"}}))
        .collect();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"p","digest":"d"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":entries})).unwrap();
    let refs = (commands.iter().enumerate())
        .map(|(n, command)| FrozenCommandRef {
            acceptance_id: format!("AC-X-00{n}"),
            kind: AcceptanceCommandKind::Command,
            chain_digest: "chain".into(),
            command_digest: content_digest(command.as_bytes()),
        })
        .collect();
    (t, policy, commit, contract, refs)
}

fn told() -> (Arc<Mutex<Vec<CheckResult>>>, CheckHook) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    (
        seen,
        Arc::new(move |result: &CheckResult| sink.lock().unwrap().push(result.clone())),
    )
}

#[tokio::test]
async fn each_result_is_told_as_it_finishes_and_a_spent_budget_defers_the_rest() {
    let (t, policy, commit, contract, refs) =
        fixture(&["test -f data/value", "test -f input", "test -s input"]);
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    let (seen, on_check) = told();
    let hooks = ObserveHooks {
        // Room for one check, then the budget is gone.
        allowance: Some(Arc::new(move || match counter.fetch_add(1, SeqCst) {
            0 => CheckAllowance::Run {
                timeout_secs: 10,
                cut: false,
            },
            _ => CheckAllowance::Defer,
        })),
        on_check: Some(on_check),
    };
    let out = observe_commands_hooked(
        &policy,
        &commit,
        &contract,
        "chain",
        &refs,
        &t.path().join("evidence"),
        Arc::new(AtomicBool::new(false)),
        &hooks,
    )
    .await
    .unwrap();
    assert!(out.operational_errors.is_empty(), "{out:?}");
    assert!(out.live_roots_unchanged && out.teardown_verified);
    assert_eq!(out.checks[0].exit_code, Some(0));
    for later in &out.checks[1..] {
        assert_eq!(later.operational_error.as_deref(), Some(CHECK_DEFERRED));
    }
    assert_eq!(out.checks.len(), 3);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "only the check that ran is told");
    assert_eq!(seen[0].acceptance_id, "AC-X-000");
    assert_eq!(seen[0].exit_code, Some(0));
    assert_eq!(
        asked.load(SeqCst),
        2,
        "the budget is asked before each start"
    );
}

#[tokio::test]
async fn a_check_cut_by_the_budget_is_deferred_and_one_past_its_cap_timed_out() {
    let (t, policy, commit, contract, refs) =
        fixture(&["sleep 20; test -f input", "test -f input"]);
    for (cut, expected) in [(true, CHECK_DEFERRED), (false, CHECK_TIMED_OUT)] {
        let (seen, on_check) = told();
        let hooks = ObserveHooks {
            allowance: Some(Arc::new(move || CheckAllowance::Run {
                timeout_secs: 1,
                cut,
            })),
            on_check: Some(on_check),
        };
        let out = observe_commands_hooked(
            &policy,
            &commit,
            &contract,
            "chain",
            &refs,
            &t.path().join(format!("evidence-{cut}")),
            Arc::new(AtomicBool::new(false)),
            &hooks,
        )
        .await
        .unwrap();
        assert!(out.operational_errors.is_empty(), "{out:?}");
        assert_eq!(out.checks[0].operational_error.as_deref(), Some(expected));
        let second = &out.checks[1];
        if cut {
            assert_eq!(second.operational_error.as_deref(), Some(CHECK_DEFERRED));
            assert_eq!(seen.lock().unwrap().len(), 1, "nothing after the cut ran");
        } else {
            assert_eq!(
                second.exit_code,
                Some(0),
                "a timed-out check stops nothing: {second:?}"
            );
            assert_eq!(seen.lock().unwrap().len(), 2);
        }
    }
}
