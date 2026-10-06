#![cfg(unix)]
use crate::command::registry::CommandHandler;
use archon_core::config::{AcceptanceExecutionConfig, ArchonConfig};
use archon_core::config_layers::{ConfigLayer, load_layered_config};
use archon_workflow::{
    WorkflowBundle, WorkflowBundleOrigin, WorkflowCommandRegistry, WorkflowSpec, WorkflowStore,
};
use std::{path::Path, sync::Arc};

struct NoNetwork;
#[async_trait::async_trait]
impl archon_pipeline::runner::LlmClient for NoNetwork {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        anyhow::bail!("this saved workflow must not call a provider")
    }
}

fn config(root: &Path, forward: bool) -> AcceptanceExecutionConfig {
    AcceptanceExecutionConfig {
        repository: root.into(),
        scratch_parent: root.join("scratch"),
        project_inputs: vec![],
        project_input_excludes: vec![],
        project_repository_view: Default::default(),
        toolchain_path: "/usr/bin:/bin".into(),
        environment: Default::default(),
        environment_allowlist: if forward {
            vec!["FIXTURE_API_KEY".into()]
        } else {
            vec![]
        },
        cargo_seed: None,
        timeout_secs: 10,
        output_bytes: 1024,
        scratch_bytes: 1024,
        external_data_roots: vec![],
    }
}

fn write_policy(path: &Path, policy: &AcceptanceExecutionConfig) {
    std::fs::write(
        path,
        format!(
            "[workflow.acceptance_execution]\n{}",
            toml::to_string(policy).unwrap()
        ),
    )
    .unwrap();
}

#[path = "acceptance_check_policy_r3_trace_tests.rs"]
mod trace;

async fn case(id: &str, mode: u8) {
    if std::env::var("R3_TUI_CASE").as_deref() != Ok(id) {
        let root = tempfile::tempdir().unwrap();
        let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([id, "--nocapture"])
            .env_clear()
            .env("R3_TUI_CASE", id)
            .env("HOME", root.path())
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("CARGO_BUILD_JOBS", "2")
            .env("RUST_TEST_THREADS", "4")
            .env("FIXTURE_API_KEY", "fixture")
            .env("OPENAI_API_KEY", "embedding-fixture")
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
    std::fs::create_dir_all(root.join(".archon")).unwrap();
    let user = root.join("user.toml");
    write_policy(&user, &config(&root, false));
    write_policy(
        &root.join(".archon/config.toml"),
        &config(&root, mode == 0 || mode == 3),
    );
    let settings = root.join("settings.toml");
    if mode == 1 {
        write_policy(
            &root.join(".archon/config.local.toml"),
            &config(&root, true),
        );
    }
    if mode == 2 {
        write_policy(
            &root.join(".archon/config.local.toml"),
            &config(&root, false),
        );
        write_policy(&settings, &config(&root, true));
    }
    let resolved = load_layered_config(
        Some(&user),
        &root,
        (mode == 2).then_some(settings.as_path()),
        (mode == 0).then_some(&[ConfigLayer::User][..]),
    )
    .unwrap();
    let expected = mode != 0;
    let store = WorkflowStore::project(&root);
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "policy-fixture".into(),
        task: "inspect fixture".into(),
        target_repository_root: Some(root.display().to_string()),
        max_agents: 1,
        max_parallelism: 1,
        stages: vec![
            serde_json::from_value(
                serde_json::json!({"id":"fixture", "kind":"agent", "task":"inspect fixture"}),
            )
            .unwrap(),
        ],
        permissions: Default::default(),
        learning_hooks: vec![],
    };
    let seed = store.create_run(spec).unwrap();
    WorkflowBundle::create_for_run(
        &store,
        &seed,
        "export default async function(w) { await w.checkpoint('fixture', {}); }",
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    WorkflowCommandRegistry::project(&root)
        .save_run("policy-fixture", &store, &seed)
        .unwrap();
    let (mut ctx, mut rx) = crate::command::test_support::CtxBuilder::new().build();
    ctx.working_dir = Some(root.clone());
    ctx.config_path = Some(user.clone());
    ctx.workflow_config = Some(resolved.clone());
    ctx.llm_adapter = Some(Arc::new(NoNetwork));
    crate::command::workflow::WorkflowHandler
        .execute(&mut ctx, &["run-template".into(), "policy-fixture".into()])
        .unwrap();
    // Await the actual asynchronous handler through its completion event.
    loop {
        let event = rx
            .recv()
            .await
            .expect("workflow notification channel closed");
        if let archon_tui::app::TuiEvent::TextDelta(text) = event {
            if text.contains("approval") || text.contains("Workflow failed") {
                break;
            }
        }
    }
    let launched = store
        .list_runs()
        .unwrap()
        .into_iter()
        .find(|run| run.id != seed.id)
        .expect("TUI launch");
    let policy = archon_workflow::acceptance_check_environment::policy_for_run(Some(
        &store.run_dir(&launched.id),
    ))
    .unwrap()
    .unwrap();
    let output =
        archon_workflow::acceptance_check_environment::CommandEnvironment::capture(Some(&policy))
            .unwrap()
            .command("/bin/sh")
            .args(["-c", "test -n \"${FIXTURE_API_KEY-}\""])
            .output()
            .unwrap();
    assert_eq!(
        output.status.success(),
        expected,
        "TUI workflow used an unresolved forwarding policy"
    );
    trace::execute(&root, &user, resolved, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r3_tui_excluded_project() {
    case("r3_tui_excluded_project", 0).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r3_tui_local_override() {
    case("r3_tui_local_override", 1).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r3_tui_settings_override() {
    case("r3_tui_settings_override", 2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r3_tui_project_policy() {
    case("r3_tui_project_policy", 3).await;
}
