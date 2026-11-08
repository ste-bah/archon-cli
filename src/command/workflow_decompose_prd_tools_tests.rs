use super::*;
use crate::command::workflow_decompose::dry_run_fixed_decomposition;

struct ToolIntakeBarrier(AtomicUsize);
#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for ToolIntakeBarrier {
    async fn build_client(
        &self,
        _: WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(archon_workflow::WorkflowError::port(
            "tool intake provider barrier",
        ))
    }
}
async fn launch_tools(project: &Path, factory: &ToolIntakeBarrier) -> anyhow::Result<String> {
    run_fixed_decomposition_with_factory(
        project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project),
        &empty_env(),
        factory,
    )
    .await
}
fn config_tools(project: &Path) {
    std::fs::write(
        project.join(".mcp.json"),
        r#"{"mcpServers":{"tradingview":{"toolPolicy":{"toolPermissions":{"probe":"safe"}}}}}"#,
    )
    .unwrap();
}
fn store_tools(project: &Path) -> WorkflowStore {
    WorkflowStore::project(archon_shell::paths::plain(project.canonicalize().unwrap()))
}

#[tokio::test]
async fn fresh_tool_refusal_precedes_provider_claim_and_run_creation() {
    for suffix in [
        "\n## 2. Required Tools\nmcp__tradingview__absent\n",
        "\n- REQ-X-002: call mcp__tradingview__absent\n",
        "\n## Required Tools\nmcp__tradingview__absent\n",
    ] {
        let project = fixture_project();
        config_tools(project.path());
        let prd = project.path().join("prds/PRD-X.md");
        let text = std::fs::read_to_string(&prd).unwrap();
        std::fs::write(&prd, format!("{text}{suffix}")).unwrap();
        let factory = ToolIntakeBarrier(AtomicUsize::new(0));
        let error = launch_tools(project.path(), &factory)
            .await
            .unwrap_err()
            .to_string();
        let path = archon_shell::paths::plain(prd.canonicalize().unwrap());
        let config =
            archon_shell::paths::plain(project.path().canonicalize().unwrap()).join(".mcp.json");
        let expected = archon_workflow::prd_tools::PrdToolDefect::NotPermitted {
            token: "mcp__tradingview__absent".into(),
        }
        .message(&path, &config);
        assert_eq!(error, expected);
        assert_eq!(factory.0.load(Ordering::SeqCst), 0);
        assert!(!project.path().join(".archon/workflows").exists());
        assert!(
            std::fs::read_dir(project.path().join("tasks/PRD-X"))
                .unwrap()
                .next()
                .is_none()
        );
    }
}

#[tokio::test]
async fn resume_and_dry_run_refuse_identically_before_author_or_new_claim() {
    let factory = ToolIntakeBarrier(AtomicUsize::new(0));
    for token in [
        "mcp__tradingview__probe",
        "mcp__tradingview__inspect",
        "mcp__tradingview__fetch",
    ] {
        let project = fixture_project();
        std::fs::write(project.path().join(".mcp.json"), serde_json::to_vec(&serde_json::json!({"mcpServers":{"tradingview":{"toolPolicy":{"toolPermissions":{token:"safe"}}}}})).unwrap()).unwrap();
        let path = project.path().join("prds/PRD-X.md");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n## 2. Required Tools\n{token}\n")).unwrap();
        let _ = launch_tools(project.path(), &factory).await.unwrap_err();
        let store = store_tools(project.path());
        let run = store.list_runs().unwrap().pop().unwrap();
        std::fs::write(project.path().join(".mcp.json"), "{}").unwrap();
        let builds = factory.0.load(Ordering::SeqCst);
        let expected = crate::command::workflow_decompose_prd_tools::validate_prd_launch_input(
            &archon_shell::paths::plain(path.canonicalize().unwrap()),
            &archon_shell::paths::plain(project.path().canonicalize().unwrap()),
        )
        .unwrap_err()
        .to_string();
        let resume = resume_fixed_decomposition_with_factory(
            project.path(),
            &run.id,
            true,
            &launch_config(project.path()),
            &empty_env(),
            &factory,
        )
        .await
        .unwrap_err()
        .to_string();
        let dry =
            dry_run_fixed_decomposition(project.path(), &run.id, &launch_config(project.path()))
                .await
                .unwrap_err()
                .to_string();
        assert_eq!(resume, expected);
        assert_eq!(dry, expected);
        assert_eq!(factory.0.load(Ordering::SeqCst), builds);
        assert_eq!(store.list_runs().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn r18_launch_resume_and_dry_run_pass_tool_intake_without_prd_changes() {
    let project = fixture_project();
    let prd = include_str!("../../tests/fixtures/r18_plan/prd.md");
    std::fs::write(project.path().join("prds/PRD-X.md"), prd).unwrap();
    config_tools(project.path());
    let factory = ToolIntakeBarrier(AtomicUsize::new(0));
    let launch = launch_tools(project.path(), &factory).await.unwrap_err();
    assert!(
        format!("{launch:#}").contains("tool intake provider barrier"),
        "{launch:#}"
    );
    let store = store_tools(project.path());
    let run = store.list_runs().unwrap().pop().unwrap();
    let resume = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{resume:#}").contains("tool intake provider barrier"),
        "{resume:#}"
    );
    let dry =
        dry_run_fixed_decomposition(project.path(), &run.id, &launch_config(project.path())).await;
    assert!(dry.is_ok(), "{dry:?}");
    assert_eq!(factory.0.load(Ordering::SeqCst), 2);
    assert_eq!(
        std::fs::read_to_string(project.path().join("prds/PRD-X.md")).unwrap(),
        prd
    );
}
