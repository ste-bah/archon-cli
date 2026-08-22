//! #201 Phase 4: the workflow CLI runs in the world its config names.
//!
//! `w.agent()`, `w.agents()`, `w.parallel()` and `w.pipeline()` all bottom out
//! in a subagent spawned from the context `build_subagent_pipeline_adapter`
//! composes, and a child inherits that context wholesale. So whether a workflow
//! is sandboxed at all is decided entirely by the two fields asserted here —
//! everything downstream is inheritance.

use super::*;
use archon_core::config::ArchonConfig;
use archon_permissions::ToolCapability;

fn docker_config() -> ArchonConfig {
    let mut config = ArchonConfig::default();
    config.sandbox.backend = "docker".into();
    // Not the default `risky`: under that mode only `Bash` and `Shell` are
    // delegated to the backend, so a `check` on anything else answers `Ok`
    // whether or not a real backend is installed.
    config.sandbox.mode = "all".into();
    config.sandbox.docker.enabled = true;
    config
}

/// Before this, `sandbox` came from `AgentConfig::default()` — `None` — so a
/// configured docker backend produced a workflow that ran every stage on the
/// host and said nothing.
#[test]
fn a_configured_backend_reaches_the_workflow_clis_agent_config() {
    let dir = tempfile::tempdir().expect("tempdir");

    let agent_config = workflow_cli_agent_config(&docker_config(), dir.path(), "workflow-run")
        .expect("a docker workspace resolves");

    let backend = agent_config
        .sandbox
        .as_ref()
        .expect("the workflow CLI ran with no sandbox backend");
    assert!(
        backend
            .check("lsp", ToolCapability::HOST_HANDLE, &serde_json::json!({}))
            .is_err(),
        "the installed backend does not gate, so it is not the isolation one"
    );
}

/// The filesystem has to arrive with the backend. A stage that reads the host
/// while its `Bash` runs in the container is the split the issue opens with,
/// and the container's own path vocabulary is what proves which one answered.
#[tokio::test]
async fn the_workflow_clis_filesystem_is_the_containers_world() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("stage_input.txt"), b"mounted").expect("seed workspace");

    let agent_config = workflow_cli_agent_config(&docker_config(), dir.path(), "workflow-run")
        .expect("a docker workspace resolves");

    let fs = agent_config
        .fs
        .as_ref()
        .expect("the workflow CLI fell back to the host filesystem");
    assert_eq!(
        fs.read(std::path::Path::new("/workspace/stage_input.txt"))
            .await
            .expect("the path a stage's own container would print"),
        b"mounted"
    );
}

/// With no isolation configured nothing changes: the workflow CLI has no
/// `/sandbox` toggle to consult, so the honest answer is the host.
#[test]
fn no_isolation_configured_leaves_the_workflow_cli_on_the_host() {
    let dir = tempfile::tempdir().expect("tempdir");

    let agent_config =
        workflow_cli_agent_config(&ArchonConfig::default(), dir.path(), "workflow-run")
            .expect("the default configuration resolves");

    assert!(agent_config.sandbox.is_none());
    assert!(agent_config.fs.is_none());
}

/// The read-before-edit policy is a third field that was left to the default,
/// and the default is `Block`. So a workflow stage enforced the strictest
/// setting whatever the config said, and someone who turned it off would find
/// their stages still refusing writes with no way to see why.
///
/// Asserted with `off` rather than `warn` because `off` is the one that must
/// travel: it is the setting whose whole purpose is that the guard is not
/// consulted at all, and a defaulted field defeats it completely.
#[test]
fn a_configured_read_before_edit_policy_reaches_a_workflow_stage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = ArchonConfig::default();
    config.filesystem.read_before_edit = archon_core::config::ReadBeforeEdit::Off;

    let agent_config = workflow_cli_agent_config(&config, dir.path(), "workflow-run")
        .expect("the default backend resolves");

    assert!(
        !agent_config.filesystem.enforces_freshness(),
        "a workflow stage still enforces read-before-edit after it was turned off"
    );
}

/// And the default is untouched when nothing is configured, so this does not
/// become a way to lose the guard by accident.
#[test]
fn an_unconfigured_read_before_edit_policy_still_blocks() {
    let dir = tempfile::tempdir().expect("tempdir");

    let agent_config =
        workflow_cli_agent_config(&ArchonConfig::default(), dir.path(), "workflow-run")
            .expect("the default configuration resolves");

    assert!(agent_config.filesystem.enforces_freshness());
}

/// An unroutable remote workspace fails the call. Falling back to the host
/// would hand the run a filesystem that disagrees with its own shell, which is
/// worse than not starting.
#[test]
fn a_workspace_that_cannot_be_reached_fails_the_call() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = ArchonConfig::default();
    config.sandbox.backend = "ssh".into();

    let error = workflow_cli_agent_config(&config, dir.path(), "workflow-run")
        .expect_err("a disabled ssh backend cannot supply a workspace");

    assert!(
        error.to_string().contains("sandbox filesystem unavailable"),
        "{error}"
    );
}
