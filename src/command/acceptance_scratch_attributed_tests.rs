#[path = "../../crates/archon-workflow/tests/support/isolation_fixture.rs"]
pub(crate) mod support;

pub(crate) struct ClearImmutable(pub(crate) std::path::PathBuf);

impl Drop for ClearImmutable {
    fn drop(&mut self) {
        let _ = std::process::Command::new("chflags")
            .args(["-R", "nouchg"])
            .arg(&self.0)
            .status();
    }
}

pub(crate) async fn attributed_observation() -> (
    tempfile::TempDir,
    ClearImmutable,
    archon_workflow::acceptance_scratch::ObservationResult,
) {
    use archon_workflow::acceptance_scratch::{CheckAllowance, ObserveHooks};
    let mut clear = None;
    let mut hooks = ObserveHooks::for_resource_limits(Default::default());
    hooks.allowance = Some(std::sync::Arc::new(|| CheckAllowance::Run {
        timeout_secs: 1,
        cut: false,
    }));
    let (temp, result) = support::observe_hooked(
        &[
            ("sleep 30", "repo_root"),
            (
                "mkdir blocked && touch blocked/keep && chflags uchg blocked/keep && sleep 30",
                "project_root",
            ),
        ],
        |root, _| clear = Some(ClearImmutable(root.join("scratch"))),
        &hooks,
    )
    .await;
    (
        temp,
        clear.expect("guard was installed before observation"),
        result,
    )
}

pub(crate) async fn unattributed_observation() -> (
    tempfile::TempDir,
    ClearImmutable,
    archon_workflow::acceptance_scratch::ObservationResult,
) {
    use archon_workflow::acceptance_scratch::{CheckAllowance, ObserveHooks};
    let mut hooks = ObserveHooks::for_resource_limits(Default::default());
    hooks.allowance = Some(std::sync::Arc::new(|| CheckAllowance::Run {
        timeout_secs: 1,
        cut: false,
    }));
    let (temp, policy, commit, contract, refs) =
        support::fixture(&[("test -f data/value", "project_root")]);
    let scratch_parent = policy.scratch_parent.clone();
    hooks.on_check = Some(std::sync::Arc::new(move |_| {
        let site = std::fs::read_dir(&scratch_parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("observation-"))
            })
            .expect("the check has a prepared site");
        let leftover = site.join("host-leftover");
        std::fs::write(&leftover, b"unowned").unwrap();
        assert!(
            std::process::Command::new("chflags")
                .arg("uchg")
                .arg(leftover)
                .status()
                .unwrap()
                .success()
        );
    }));
    let result = archon_workflow::acceptance_scratch::observe_commands_hooked(
        &policy,
        &commit,
        &contract,
        "chain",
        &refs,
        &temp.path().join("evidence"),
        archon_workflow::acceptance_scratch::ObserveControl::new(
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            hooks,
        ),
    )
    .await
    .unwrap();
    let clear = ClearImmutable(policy.scratch_parent);
    (temp, clear, result)
}

pub(crate) fn assert_owner_finding(
    result: &archon_workflow::acceptance_scratch::ObservationResult,
) {
    let teardown = result
        .attributed_teardown
        .as_ref()
        .expect("the fixture must produce an attributed teardown");
    let owner = result
        .checks
        .iter()
        .find(|check| check.acceptance_id == teardown.acceptance_id)
        .expect("the owning check is retained");
    assert!(owner.operational_error.is_some(), "{owner:?}");
    assert_eq!(
        owner.fault,
        Some(archon_workflow::acceptance_scratch::faults::FaultCause::SiteTeardownFailed),
        "{owner:?}"
    );
}
