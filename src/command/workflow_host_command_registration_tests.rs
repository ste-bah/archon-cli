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

async fn no_unrecorded_execution(mode: &str) {
    use crate::command::workflow_host_command_groups::{
        GROUP_RECORDS_DIR, REGISTER_DELAY, require_no_running_groups,
    };
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    match mode {
        "readonly" => {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap()
        }
        "obstruction" => {
            std::fs::remove_dir(&dir).unwrap();
            std::fs::write(&dir, b"blocked").unwrap();
        }
        _ => {
            std::fs::remove_dir(&dir).unwrap();
            std::fs::remove_dir(dir.parent().unwrap()).unwrap();
            std::fs::write(dir.parent().unwrap(), b"blocked").unwrap();
        }
    }
    let request = ResolvedHostCommand {
        command_id: "fixture".into(), program: "python3".into(),
        args: vec!["-c".into(), "import subprocess,time; p=subprocess.Popen(['sleep','30'],start_new_session=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); open('escaped','w').write(str(p.pid)); time.sleep(30)".into()],
        cwd: run.path().into(), environment: std::env::vars_os().filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value))).collect(),
        stdin: None, timeout_secs: 30, max_stdout_bytes: 4096, max_stderr_bytes: 4096,
        declared_write_set: Vec::new(), remediation_scopes: Default::default(),
        spill_dir: None,
    };
    let (control, _handle) = HostCommandControl::new();
    REGISTER_DELAY.with(|delay| delay.set(true));
    let result = crate::command::workflow_host_command_supervisor::supervise_process_group(
        request,
        control,
        Some(&dir),
    )
    .await;
    REGISTER_DELAY.with(|delay| delay.set(false));
    let executed = run.path().join("escaped").exists();
    if executed {
        let pid: u32 = std::fs::read_to_string(run.path().join("escaped"))
            .unwrap()
            .parse()
            .unwrap();
        if let Ok(Some(start)) = archon_shell::process_tree::identity_of(pid) {
            archon_shell::process_tree::deliver(
                archon_shell::process_tree::Pinned { pid, start },
                libc::SIGKILL,
            );
        }
    }
    match mode {
        "readonly" => {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap()
        }
        "obstruction" => {
            std::fs::remove_file(&dir).unwrap();
            std::fs::create_dir(&dir).unwrap();
        }
        _ => {
            std::fs::remove_file(dir.parent().unwrap()).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
        }
    }
    assert!(result.is_err());
    assert!(require_no_running_groups(run.path(), "run").is_ok());
    assert!(
        !executed,
        "{mode}: an unregistered child escaped before the registration failure paused"
    );
}

#[tokio::test]
async fn readonly_directory_prevents_unrecorded_execution() {
    no_unrecorded_execution("readonly").await;
}
#[tokio::test]
async fn obstructed_directory_prevents_unrecorded_execution() {
    no_unrecorded_execution("obstruction").await;
}
#[tokio::test]
async fn obstructed_parent_prevents_unrecorded_execution() {
    no_unrecorded_execution("parent").await;
}
