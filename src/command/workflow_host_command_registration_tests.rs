use super::*;
use std::os::unix::fs::PermissionsExt;

struct RegisteringProcess(std::path::PathBuf);
#[async_trait::async_trait]
impl HostCommandProcessAdapter for RegisteringProcess {
    async fn execute(
        &self,
        mut request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        request.program = "/bin/sh".into();
        request.args = vec!["-c".into(), "exit 0".into()];
        crate::command::workflow_host_command_supervisor::supervise_process_group(
            request,
            control,
            Some(&self.0),
        )
        .await
    }
}

async fn registration_failure(mode: &str) {
    let fixture = fixture(vec![]);
    let run_root = fixture.store.run_dir(&fixture.run_id);
    let dir = if mode == "missing-parent" {
        run_root.join("blocked-parent").join("records")
    } else {
        run_root.join(crate::command::workflow_host_command_groups::GROUP_RECORDS_DIR)
    };
    if mode != "missing-parent" {
        std::fs::create_dir_all(&dir).unwrap();
    }
    match mode {
        "readonly" => {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap()
        }
        "not-directory" => {
            std::fs::remove_dir(&dir).unwrap();
            std::fs::write(&dir, b"obstruction").unwrap();
        }
        _ => {
            std::fs::write(dir.parent().unwrap(), b"obstruction").unwrap();
        }
    }
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        fixture.context.clone(),
        run_root,
        Arc::new(RegisteringProcess(dir.clone())),
    );
    let result = executor.execute(lint(), Some(fixture.generation)).await;
    if mode == "readonly" {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let error = result.unwrap_err();
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{mode}: {error}"
    );
    let note = error.to_string();
    assert!(note.contains(&dir.display().to_string()), "{note}");
    assert!(
        note.contains(&format!(
            "archon workflow resume --live --yes {}",
            fixture.run_id
        )),
        "{note}"
    );
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
}

#[tokio::test]
async fn readonly_registration_pauses() {
    registration_failure("readonly").await;
}
#[tokio::test]
async fn obstructed_registration_pauses() {
    registration_failure("not-directory").await;
}
#[tokio::test]
async fn obstructed_parent_registration_pauses() {
    registration_failure("missing-parent").await;
}
