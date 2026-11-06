use super::*;

#[tokio::test]
async fn repair_uses_run_bound_and_forwarded_names_without_rereading_config() {
    let run = run_fixture_with(&[("AC-X-001", "set -u; test -f \"$INPUT_DATA\"", true)]);
    let mut context = super::super::exec::resolve_context(
        &run.store,
        &run.run_id,
        run.runtime.target_repository_root.as_deref(),
        Some(&run.universe),
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .unwrap();
    let contract = run.set.contract();
    // The project has no configured allowlist. Only the captured run policy
    // authorizes this name; rereading disk would incorrectly seed a repair.
    for forwarded in [false, true] {
        let environment = if forwarded {
            serde_json::json!({})
        } else {
            serde_json::json!({"INPUT_DATA": "input"})
        };
        let allowlist = if forwarded {
            vec!["INPUT_DATA"]
        } else {
            vec![]
        };
        context.binding = Some(
            serde_json::from_value(serde_json::json!({
                "source_commit": "0".repeat(40),
                "policy": {
                    "repository": context.repository, "project": context.project,
                    "task_root": context.task_root, "scratch_parent": "/scratch",
                    "project_inputs": [], "combined": false, "toolchain_path": "/bin",
                    "environment": environment, "environment_allowlist": allowlist,
                    "cargo_seed": null, "timeout_secs": 60, "output_bytes": 4096,
                    "scratch_bytes": 1048576
                }
            }))
            .unwrap(),
        );
        assert!(
            super::super::repair::repair_unaccepted(None, &context, &contract, None)
                .await
                .unwrap()
                .is_none()
        );
    }
}
