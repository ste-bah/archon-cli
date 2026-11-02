use super::*;

#[tokio::test]
async fn host_command_hermetic_direct_setup_keeps_transient_io() {
    use crate::command::workflow_task_set::executability::probe_tests::trees;
    let trees = trees(&[(
        "AC-X-001",
        "test -f feature.txt",
        archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
    )]);
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_owned());
    let fault = crate::command::workflow_host_fault_test_hooks::inject(
        crate::command::workflow_host_fault_test_hooks::Point::Direct,
        &trees.repo,
        Box::new(|| Err(std::io::Error::from_raw_os_error(5))),
    );
    let contract = trees.contract();
    let digest = contract_digest(&contract).unwrap();
    let refs = refs_for(&contract, &digest, &trees.ids());
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        run_in_copy(
            &probe,
            &trees.repo,
            &trees.base,
            &contract,
            &digest,
            &refs,
            &archon_workflow::acceptance_scratch::ObserveHooks::for_resource_limits(
                Default::default(),
            ),
            &Default::default(),
        ),
    )
    .await
    .expect("probe must end");
    assert!(fault.fired());
    let error = result.err().expect("direct setup failed");
    assert_eq!(
        error.1,
        crate::command::workflow_host_fault::Class::Transient,
        "{}",
        error.0
    );
}
