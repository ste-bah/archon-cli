// The multi-check observation fixture of the acceptance_scratch isolation
// targets: a repository, a project beside it, and one accepted check per
// command.
#![allow(dead_code)]

pub use archon_workflow::acceptance_scratch::faults::{CheckFault, check_fault};
use archon_workflow::acceptance_scratch::{
    ObservationResult, ObserveControl, ObserveHooks, ScratchPolicy, observe_commands,
    observe_commands_hooked,
};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
pub use std::{collections::BTreeMap, path::Path, process::Command};

pub fn git(root: &Path, args: &[&str]) -> String {
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
pub fn fixture(
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

pub async fn observe(checks: &[(&str, &str)]) -> (tempfile::TempDir, ObservationResult) {
    observe_with(checks, |_, _| {}).await
}

/// [`observe`] with the policy changed by `change` (given the fixture root).
pub async fn observe_with(
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

pub fn is_checks_own(error: Option<&str>) -> bool {
    error.is_some_and(|error| matches!(check_fault(error), CheckFault::Check { .. }))
}

/// The other checks' verdicts, and an observation that is not void.
pub fn assert_siblings_judged(result: &ObservationResult, offender: usize) {
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

/// [`observe_with`] under `hooks`, the policy changed by `change`.
pub async fn observe_hooked(
    checks: &[(&str, &str)],
    change: impl FnOnce(&Path, &mut ScratchPolicy),
    hooks: &ObserveHooks,
) -> (tempfile::TempDir, ObservationResult) {
    let (t, mut policy, commit, contract, refs) = fixture(checks);
    change(t.path(), &mut policy);
    let result = observe_commands_hooked(
        &policy,
        &commit,
        &contract,
        "chain",
        &refs,
        &t.path().join("evidence"),
        ObserveControl::new(
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            hooks.clone(),
        ),
    )
    .await
    .unwrap();
    (t, result)
}

/// The build cache generation of the fixture rooted at `root` whose policy
/// set `build_cache` to `root/cache`: a new one means every build was forgotten.
pub fn generation(root: &Path) -> u64 {
    std::fs::read_to_string(root.join("cache/generation"))
        .map_or(0, |text| text.trim().parse().unwrap())
}
