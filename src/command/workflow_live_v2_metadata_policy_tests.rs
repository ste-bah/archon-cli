#![cfg(unix)]

use super::*;
use archon_workflow::acceptance_check_environment::{CommandEnvironment, policy_for_run};

fn operator(case: &str, kind: u8) {
    if std::env::var("ISSUE_349_METADATA_CASE").as_deref() != Ok(case) {
        let root = tempfile::tempdir().unwrap();
        let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([case, "--nocapture"])
            .env_clear()
            .env("ISSUE_349_METADATA_CASE", case)
            .env("HOME", root.path())
            .env("PATH", "/usr/bin:/bin")
            .env("RUST_TEST_THREADS", "4")
            .env("FIXTURE_API_KEY", "fixture-data")
            .env("OPERATOR_SECRET", "withheld")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let config_path = root.join("operator.toml");
    let config = archon_core::config::AcceptanceExecutionConfig {
        repository: root.clone(),
        scratch_parent: root.parent().unwrap().join("scratch"),
        project_inputs: vec![],
        project_input_excludes: vec![],
        project_repository_view: Default::default(),
        toolchain_path: "/usr/bin:/bin".into(),
        environment: Default::default(),
        environment_allowlist: vec!["FIXTURE_API_KEY".into()],
        cargo_seed: None,
        timeout_secs: 10,
        output_bytes: 1024,
        scratch_bytes: 1024,
        external_data_roots: vec![],
    };
    std::fs::write(
        &config_path,
        format!(
            "[workflow.acceptance_execution]\n{}",
            toml::to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let action = if kind == 1 {
        archon_workflow::CommandAction::RunTemplate {
            name: "verifier".into(),
            args: None,
        }
    } else {
        archon_workflow::CommandAction::Run {
            task: "verify files".into(),
            decomposed: true,
        }
    };
    let mut resolved = archon_core::config::ArchonConfig::default();
    resolved.workflow.acceptance_execution = Some(config);
    let binding = if kind == 2 {
        Some(crate::command::acceptance_check_policy::from_config(&resolved).unwrap())
    } else {
        crate::command::acceptance_check_policy::for_new_plan(&action, &root, Some(&config_path))
            .unwrap()
    };
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "verifier".into(),
        task: "verify files".into(),
        target_repository_root: Some(root.display().to_string()),
        max_agents: 1,
        max_parallelism: 1,
        stages: vec![],
        permissions: Default::default(),
        learning_hooks: vec![],
    };
    let mut plan = if kind == 0 {
        WorkflowScriptPlan::generated(
            "verify files",
            "export default async function(w) {}",
            vec![],
            Some(archon_workflow::task_universe::WorkflowV2TaskUniverse {
                schema_version: "workflow-v2-task-universe-v1".into(),
                source_roots: vec![],
                tasks: vec![],
            }),
            Default::default(),
            &Default::default(),
        )
        .unwrap()
    } else {
        WorkflowScriptPlan::fixed(
            spec,
            "export default async function(w) {}",
            vec![],
            serde_json::json!({}),
        )
    };
    plan.check_policy = binding;
    let store = WorkflowStore::project(&root);
    let run = store.create_run(plan.approval_metadata_spec()).unwrap();
    if kind == 2 {
        let identity = FixedRunIdentityV1 {
            template_version: "fixed-decomposition-v1".into(),
            starting_binary_revision: "test".into(),
            script_digest: "script".into(),
            catalog_digest: "catalog".into(),
            project_root_identity: root.display().to_string(),
            prd_identity: root.join("input.md").display().to_string(),
            task_root_identity: root.join("tasks").display().to_string(),
        };
        save_fixed_decomposition_metadata(&store, &run.id, &plan, &identity).unwrap();
    } else {
        save_generated_v2_metadata(&store, &run.id, &plan, false).unwrap();
    }
    assert!(
        load_generated_v2_metadata(&store, &run.id)
            .unwrap()
            .unwrap()
            .observer_snapshot
            .is_none()
    );
    let policy = policy_for_run(Some(&store.run_dir(&run.id))).unwrap();
    let output = CommandEnvironment::capture(policy.as_ref())
        .unwrap()
        .command("/bin/sh")
        .args([
            "-c",
            "test \"$FIXTURE_API_KEY\" = fixture-data && test -z \"${OPERATOR_SECRET-}\"",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "configured data did not reach the {} launch verifier",
        kind
    );
    std::fs::write(&config_path, "broken = [").unwrap();
    assert!(crate::command::acceptance_check_policy::load(&root, Some(&config_path)).is_err());
    // CLI consumes its resolved config; resume consumes the launch record.
    assert!(
        crate::command::acceptance_check_policy::for_config_action(&action, &resolved)
            .unwrap()
            .unwrap()
            .is_some()
    );
    assert!(
        crate::command::acceptance_check_policy::for_new_plan(
            &archon_workflow::CommandAction::Resume {
                run_id: run.id.clone()
            },
            &root,
            Some(&config_path),
        )
        .unwrap()
        .is_none()
    );
    std::fs::write(&config_path, "workflow = 'wrong'").unwrap();
    assert!(crate::command::acceptance_check_policy::load(&root, Some(&config_path)).is_err());

    let path = store.run_dir(&run.id).join(GENERATED_V2_METADATA_PATH);
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // A valid older authority must not rescue a corrupt new policy binding.
    metadata["observer_snapshot"] = serde_json::json!({"native_execution": {"policy": {
        "repository": root, "project": root, "task_root": root.join("tasks"),
        "scratch_parent": root.parent().unwrap().join("scratch"), "project_inputs": [],
        "combined": true, "toolchain_path": "/usr/bin:/bin", "environment": {},
        "environment_allowlist": ["FIXTURE_API_KEY"], "cargo_seed": null,
        "timeout_secs": 10, "output_bytes": 1024, "scratch_bytes": 1024,
    }}});
    for invalid in [
        serde_json::json!([]),
        serde_json::json!({"toolchain_path": null, "bound": {}, "forwarded": []}),
        serde_json::json!({"toolchain_path": "/usr/bin:/bin", "bound": {"FIXTURE_API_KEY": "literal-secret"}, "forwarded": []}),
        serde_json::json!({"toolchain_path": "/usr/bin:/bin", "bound": {}, "forwarded": ["BASH_ENV"]}),
    ] {
        metadata["check_environment_policy"] = invalid;
        std::fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let error = policy_for_run(Some(&store.run_dir(&run.id))).unwrap_err();
        assert!(!error.contains("literal-secret"), "{error}");
    }
    metadata["check_environment_policy"] = serde_json::Value::Null;
    std::fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    assert!(
        policy_for_run(Some(&store.run_dir(&run.id)))
            .unwrap()
            .is_none()
    );
    metadata
        .as_object_mut()
        .unwrap()
        .remove("check_environment_policy");
    std::fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    assert!(
        policy_for_run(Some(&store.run_dir(&run.id)))
            .unwrap()
            .is_some()
    );
}

#[test]
fn review349_legacy_launch_records_policy_without_observer() {
    operator("review349_legacy_launch_records_policy_without_observer", 0);
}
#[test]
fn review349_saved_launch_records_policy_without_observer() {
    operator("review349_saved_launch_records_policy_without_observer", 1);
}
#[test]
fn review349_fixed_launch_records_policy_without_observer() {
    operator("review349_fixed_launch_records_policy_without_observer", 2);
}
