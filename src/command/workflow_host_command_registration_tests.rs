use super::*;
use std::os::unix::fs::PermissionsExt;

struct RegisteringProcess {
    path: std::path::PathBuf,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl HostCommandProcessAdapter for RegisteringProcess {
    async fn execute(
        &self,
        mut request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
            return Err(WorkflowError::StageFailed(
                "unexpected registration retry".into(),
            ));
        }
        request.program = "/bin/sh".into();
        request.args = vec!["-c".into(), "exit 0".into()];
        crate::command::workflow_host_command_supervisor::supervise_process_group(
            request,
            control,
            Some(&self.path),
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
    let process = Arc::new(RegisteringProcess {
        path: dir.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        fixture.context.clone(),
        run_root,
        process.clone(),
    );
    let result = executor.execute(lint(), Some(fixture.generation)).await;
    if mode == "readonly" {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let result = result.unwrap();
    assert_eq!(process.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let error = result.gate_envelope.unwrap().operational_error.unwrap();
    assert_eq!(error.kind, "permanent_host_fault");
    assert!(
        error.text.contains(&dir.display().to_string()),
        "{}",
        error.text
    );
    assert!(error.text.contains("remedy:"), "{}", error.text);
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Running
    );
}

#[tokio::test]
async fn readonly_registration_requires_permission_repair() {
    registration_failure("readonly").await;
}
#[tokio::test]
async fn obstructed_registration_requires_directory_repair() {
    registration_failure("not-directory").await;
}
#[tokio::test]
async fn obstructed_parent_registration_requires_directory_repair() {
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
