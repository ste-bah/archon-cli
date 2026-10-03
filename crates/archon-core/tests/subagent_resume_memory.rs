//! Memory is the only resume authority. Disk contributes conversation only.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use archon_tools::tool::ToolContext;
use harness::*;
use memory_harness::*;
use std::sync::Arc;

#[tokio::test]
async fn inherited_effective_context_survives_resume_without_metadata() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let target = project.join("read.txt");
    std::fs::write(&target, "unchanged").unwrap();
    let store = store(&root);
    let host = Host::new(
        &project,
        "memory-context",
        vec![STOP, write(&target, "changed"), read(&target), STOP],
    );
    host.spawn(
        "child",
        request(
            &workspace,
            Some("workspace-boundary"),
            vec![target.display().to_string()],
        ),
        ToolContext {
            denied_directory_names: vec!["secret".into()],
            workflow_read_guard: Some(Arc::new(
                archon_tools::workflow_read_guard::WorkflowReadGuard::new(40, 20, false, false),
            )),
            run_store: Some(Default::default()),
            ..parent(&project, &[])
        },
    )
    .await
    .unwrap();
    history(&store, "child");
    let plan = host
        .plan(&store, "child")
        .await
        .expect("memory must restore inherited context without a sidecar");
    host.resume("child", plan, ToolContext::default())
        .await
        .unwrap();
    assert!(host.outcome(1).is_error);
    assert!(!host.outcome(2).is_error);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "unchanged");
}

#[tokio::test]
async fn forged_unconfined_sidecar_is_ignored() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let store = store(&root);
    let host = Host::new(&project, "memory-forge", vec![STOP, STOP]);
    host.spawn(
        "child",
        request(&workspace, Some("workspace-boundary"), vec![]),
        parent(&project, &[]),
    )
    .await
    .unwrap();
    history(&store, "child");
    forge(&store, "child");
    let plan = host.plan(&store, "child").await.unwrap();
    assert_eq!(
        plan.request.cwd,
        Some(workspace.display().to_string()),
        "forged sidecar became authority"
    );
    host.resume("child", plan, ToolContext::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn fresh_process_refuses_even_a_forged_unconfined_record() {
    let (_t, root) = real_temp();
    let store = store(&root);
    history(&store, "never-spawned");
    forge(&store, "never-spawned");
    let host = Host::new(&root, "memory-fresh", vec![]);
    match host.plan(&store, "never-spawned").await {
        Err(error) => assert_eq!(error, unknown("never-spawned")),
        Ok(_) => panic!("fresh process accepted disk confinement"),
    }
}

#[tokio::test]
async fn stopped_id_reuse_restores_only_the_new_context() {
    let (_t, root) = real_temp();
    let first = dir(&root, "first");
    let newer = dir(&root, "newer");
    let store = store(&root);
    let host = Host::new(&root, "memory-reuse", vec![STOP, STOP, STOP]);
    host.spawn("reused", request(&first, None, vec![]), parent(&root, &[]))
        .await
        .unwrap();
    host.spawn(
        "reused",
        request(&newer, Some("workspace-boundary"), vec![]),
        parent(&root, &[]),
    )
    .await
    .unwrap();
    history(&store, "reused");
    let plan = host
        .plan(&store, "reused")
        .await
        .expect("new context must exist independently of metadata");
    assert_eq!(plan.request.cwd, Some(newer.display().to_string()));
    host.resume("reused", plan, ToolContext::default())
        .await
        .unwrap();
}

#[test]
fn final_collection_bounds_retained_agents() {
    let mut manager = archon_core::subagent::SubagentManager::new(4);
    for index in 0..300 {
        let id = format!("agent-{index}");
        manager
            .register_with_id(id.clone(), request(std::path::Path::new("/"), None, vec![]))
            .unwrap();
        manager.complete(&id, "done".into()).unwrap();
        manager.cleanup_agent(&id);
    }
    assert!(
        manager.get_status("agent-0").is_none(),
        "final collection never removed the oldest stopped agent"
    );
}

#[tokio::test]
async fn boundary_worktree_readonly_backend_and_environment_are_preserved() {
    use archon_core::{
        agent::AgentConfig,
        sandbox::{DockerConfig, DockerFs, DockerSandboxBackend},
    };
    use archon_tools::{
        isolation::AutoIsolation,
        provider_env::{ProviderEnvPolicy, ProviderEnvSource},
    };
    let (_t, root) = real_temp();
    let repo = checkout(&root);
    let read_target = root.join("read-target");
    std::fs::write(&read_target, "unchanged").unwrap();
    let profile = root.join("profile");
    let secret = "memory-test-credential-123456789";
    std::fs::write(&profile, format!("export RESUME_TEST_API_KEY='{secret}'\n")).unwrap();
    let store = store(&root);
    let probe = ("ContextProbe", serde_json::json!({}));
    let host = Host::with_config(
        &root,
        "memory-effective",
        vec![
            probe.clone(),
            STOP,
            probe,
            write(&read_target, "changed"),
            read(&read_target),
            write(std::path::Path::new("seed"), "changed"),
            STOP,
        ],
        AgentConfig {
            subagent_auto_isolation: AutoIsolation::Always,
            subagent_stream_idle_timeout_secs: 47,
            ..Default::default()
        },
    );
    let mut spawn = request(
        &repo,
        Some("workspace-boundary"),
        vec![read_target.display().to_string()],
    );
    spawn.allowed_tools.push("ContextProbe".into());
    spawn.provider_env = Some(ProviderEnvSource::Policy(ProviderEnvPolicy {
        required_keys: vec!["RESUME_TEST_API_KEY".into()],
        profile_sources: vec![profile.display().to_string()],
        reason: None,
    }));
    let sandbox = Arc::new(DockerSandboxBackend::new(
        DockerConfig {
            enabled: true,
            ..Default::default()
        },
        "ro",
        archon_permissions::SandboxScope::Session,
    ));
    host.spawn(
        "effective",
        spawn,
        ToolContext {
            sandbox: Some(sandbox.clone()),
            fs: Some(Arc::new(DockerFs::with_workspace_access(&repo, "ro", &[]))),
            turn_id: Some("original-turn".into()),
            denied_directory_names: vec!["secret".into()],
            workflow_read_guard: Some(Arc::new(
                archon_tools::workflow_read_guard::WorkflowReadGuard::new(40, 20, false, false),
            )),
            run_store: Some(Default::default()),
            ..parent(&root, &[])
        },
    )
    .await
    .unwrap();
    history(&store, "effective");
    let plan = host
        .plan(&store, "effective")
        .await
        .expect("the effective context must survive without metadata");
    host.resume("effective", plan, ToolContext::default())
        .await
        .unwrap();
    let contexts = host.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 2);
    let (before, after) = (&contexts[0], &contexts[1]);
    assert_ne!(before.working_dir, repo);
    assert_eq!(before.working_dir, after.working_dir);
    assert_eq!(before.extra_dirs, after.extra_dirs);
    assert_eq!(before.write_roots, after.write_roots);
    assert!(!before.sealed_repositories.is_empty());
    assert_eq!(before.sealed_repositories, after.sealed_repositories);
    assert_eq!(before.denied_directory_names, after.denied_directory_names);
    assert_eq!(before.mode, after.mode);
    assert_eq!(before.turn_id, after.turn_id);
    assert_eq!(before.in_fork, after.in_fork);
    assert_eq!(before.nested, after.nested);
    assert_eq!(
        format!("{:?}", before.run_store),
        format!("{:?}", after.run_store)
    );
    assert!(Arc::ptr_eq(
        before.workflow_read_guard.as_ref().unwrap(),
        after.workflow_read_guard.as_ref().unwrap()
    ));
    assert!(before.audit_landing.is_none() && after.audit_landing.is_none());
    assert!(Arc::ptr_eq(
        before.sandbox.as_ref().unwrap(),
        after.sandbox.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        before.fs.as_ref().unwrap(),
        after.fs.as_ref().unwrap()
    ));
    assert!(host.outcome(3).is_error);
    assert!(!host.outcome(4).is_error);
    assert!(host.outcome(5).is_error, "read-only filesystem was lost");
    assert_eq!(std::fs::read_to_string(&read_target).unwrap(), "unchanged");
    assert_eq!(
        std::fs::read_to_string(before.working_dir.join("seed")).unwrap(),
        "base"
    );
}

#[tokio::test]
async fn effective_host_timeout_is_not_replaced_by_the_resume_caller() {
    use archon_tools::host_timeout::{HostTimeout, scope};
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let store = store(&root);
    let host = Host::new(&root, "memory-timeout", vec![STOP, STOP]);
    let mut spawn = request(&workspace, None, vec![]);
    spawn.timeout_secs = 0;
    scope(
        HostTimeout::Unlimited,
        host.spawn("timeout", spawn, parent(&root, &[])),
    )
    .await
    .unwrap();
    history(&store, "timeout");
    forge(&store, "timeout");
    // Give the old file design a fully valid record so this test reaches its deadline logic.
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.metadata_path("timeout")).unwrap())
            .unwrap();
    meta["confinement"]["cwd"] = workspace.display().to_string().into();
    meta["confinement"]["timeout_secs"] = 0.into();
    std::fs::write(store.metadata_path("timeout"), meta.to_string()).unwrap();
    let plan = host.plan(&store, "timeout").await.unwrap();
    let result = scope(
        HostTimeout::Finite(0),
        host.resume("timeout", plan, parent(&root, &[])),
    )
    .await;
    assert!(
        result.is_ok(),
        "resume replaced the original unlimited host timeout: {result:?}"
    );
}

#[tokio::test]
async fn an_old_pending_resume_cannot_resurrect_a_reused_id() {
    let (_t, root) = real_temp();
    let first = dir(&root, "first");
    let newer = dir(&root, "newer");
    let store = store(&root);
    let host = Host::new(&root, "memory-stale", vec![STOP, STOP, STOP]);
    host.spawn("reused", request(&first, None, vec![]), parent(&root, &[]))
        .await
        .unwrap();
    history(&store, "reused");
    forge(&store, "reused");
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.metadata_path("reused")).unwrap())
            .unwrap();
    meta["confinement"]["cwd"] = first.display().to_string().into();
    std::fs::write(store.metadata_path("reused"), meta.to_string()).unwrap();
    let stale = host.plan(&store, "reused").await.unwrap();
    host.spawn("reused", request(&newer, None, vec![]), parent(&root, &[]))
        .await
        .unwrap();
    let result = host.resume("reused", stale, parent(&root, &[])).await;
    assert!(
        result.is_err(),
        "a stale plan restored the replaced context"
    );
    assert!(result.unwrap_err().to_string().contains("superseded"));
}

#[tokio::test]
async fn lowered_tier_cap_refuses_despite_a_forged_lower_rung() {
    use archon_core::agent::AgentConfig;
    use archon_tools::isolation::{AutoIsolation, IsolationTier};
    let (_t, root) = real_temp();
    let repo = checkout(&root);
    let store = store(&root);
    let first = Host::with_config(
        &root,
        "memory-cap",
        vec![STOP],
        AgentConfig {
            subagent_auto_isolation: AutoIsolation::Always,
            ..Default::default()
        },
    );
    first
        .spawn(
            "capped",
            request(&repo, Some("workspace-boundary"), vec![]),
            parent(&root, &[]),
        )
        .await
        .unwrap();
    history(&store, "capped");
    forge(&store, "capped");
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.metadata_path("capped")).unwrap())
            .unwrap();
    meta["confinement"]["cwd"] = repo.display().to_string().into();
    std::fs::write(store.metadata_path("capped"), meta.to_string()).unwrap();
    let capped = Host::with_manager(
        &root,
        "memory-cap",
        vec![STOP],
        AgentConfig {
            subagent_isolation_max_tier: IsolationTier::Shared,
            ..Default::default()
        },
        first.manager.clone(),
    );
    let plan = capped.plan(&store, "capped").await.unwrap();
    let result = capped.resume("capped", plan, parent(&root, &[])).await;
    assert!(
        result.is_err(),
        "forged lower rung bypassed the current tier cap"
    );
    let refusal = result.unwrap_err().to_string();
    assert!(
        refusal.contains("capped")
            && refusal.contains("worktree")
            && refusal.contains("isolation_max_tier"),
        "{refusal}"
    );
}

#[tokio::test]
async fn provider_overlay_and_redaction_are_frozen_at_spawn() {
    use archon_tools::provider_env::{ProviderEnvPolicy, ProviderEnvSource};
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let profile = root.join("profile");
    let secret = "resume-fixture-secret-987654321";
    std::fs::write(
        &profile,
        format!("export RESUME_FIXTURE_API_KEY='{secret}'\n"),
    )
    .unwrap();
    let store = store(&root);
    let command = (
        "Bash",
        serde_json::json!({"command":"printf '%s' \"$RESUME_FIXTURE_API_KEY\""}),
    );
    let host = Host::new(
        &root,
        "memory-env",
        vec![command.clone(), STOP, command, STOP],
    );
    let mut spawn = request(&workspace, Some("workspace-boundary"), vec![]);
    spawn.allowed_tools = vec!["Bash".into()];
    spawn.provider_env = Some(ProviderEnvSource::Policy(ProviderEnvPolicy {
        required_keys: vec!["RESUME_FIXTURE_API_KEY".into()],
        profile_sources: vec![profile.display().to_string()],
        reason: None,
    }));
    host.spawn("environment", spawn, parent(&root, &[]))
        .await
        .unwrap();
    history(&store, "environment");
    forge(&store, "environment");
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.metadata_path("environment")).unwrap())
            .unwrap();
    meta["confinement"]["cwd"] = workspace.display().to_string().into();
    meta["confinement"]["isolation"] = "workspace-boundary".into();
    meta["confinement"]["allowed_tools"] = serde_json::json!(["Bash"]);
    std::fs::write(store.metadata_path("environment"), meta.to_string()).unwrap();
    std::fs::write(&profile, "export RESUME_FIXTURE_API_KEY='changed-value'\n").unwrap();
    let plan = host.plan(&store, "environment").await.unwrap();
    let overlay = plan
        .request
        .provider_env
        .as_ref()
        .and_then(|source| source.resolution())
        .expect("the effective provider overlay was not retained");
    let mut values = vec![];
    overlay.apply_to_env(&mut values);
    assert_eq!(
        values,
        vec![("RESUME_FIXTURE_API_KEY".into(), secret.into())]
    );
    assert_eq!(
        overlay.redact_text(secret),
        "<redacted:RESUME_FIXTURE_API_KEY>"
    );
    host.resume("environment", plan, ToolContext::default())
        .await
        .unwrap();
    let outcome = host.outcome(2);
    assert!(
        !outcome.is_error && outcome.text.contains("<redacted:RESUME_FIXTURE_API_KEY>"),
        "{outcome:?}"
    );
    assert!(!outcome.text.contains(secret));
}

#[tokio::test]
async fn collection_drops_the_confinement_and_refuses_later_resume() {
    let (_t, root) = real_temp();
    let store = store(&root);
    let host = Host::new(&root, "memory-collection", vec![STOP]);
    host.spawn(
        "collected",
        request(&root, None, vec![]),
        parent(&root, &[]),
    )
    .await
    .unwrap();
    history(&store, "collected");
    forge(&store, "collected");
    {
        let mut manager = host.manager.lock().await;
        for index in 0..300 {
            let id = format!("temporary-{index}");
            manager
                .register_with_id(id.clone(), request(&root, None, vec![]))
                .unwrap();
            manager.complete(&id, "done".into()).unwrap();
            manager.cleanup_agent(&id);
        }
        assert!(
            manager.get_status("collected").is_none(),
            "the original context was never collected"
        );
    }
    match host.plan(&store, "collected").await {
        Err(error) => assert_eq!(error, unknown("collected")),
        Ok(_) => panic!("collected confinement was recovered from a file"),
    }
}
