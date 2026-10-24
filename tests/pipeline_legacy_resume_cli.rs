//! Legacy `pipeline resume` (a checkpoint with no bundle manifest) must run
//! every fallible precondition before it marks the checkpoint Running.
//!
//! Both tests drive the real binary in an isolated HOME with a cleared
//! environment, so no ambient credential or provider value can leak in.
#![cfg(unix)]

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use archon_pipeline::runner::PipelineType;
use archon_pipeline::session::{SessionStatus, checkpoint, new_session};

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let project = root.join("project");
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        Self {
            _tmp: tmp,
            root,
            project,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_archon"));
        command
            .current_dir(&self.project)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "dumb")
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("ARCHON_DATA_DIR", self.root.join("data").join("archon"))
            .args(args);
        command
    }

    /// Install the bundled agents so preflight passes, then save an
    /// Interrupted legacy checkpoint. Returns its path.
    fn interrupted_checkpoint(&self) -> (String, PathBuf) {
        let sync = self.command(&["agent", "sync"]).output().unwrap();
        assert!(sync.status.success(), "agent sync failed: {}", text(&sync));
        let mut session = new_session(PipelineType::Coding, "legacy resume task");
        session.status = SessionStatus::Interrupted;
        checkpoint(&session, &self.project).unwrap();
        let path = self
            .project
            .join(".pipeline-state")
            .join(&session.session_id)
            .join("checkpoint.json");
        (session.session_id, path)
    }
}

fn text(output: &Output) -> String {
    format!(
        "rc={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn status_of(path: &Path) -> String {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    value["status"].as_str().unwrap().to_string()
}

#[test]
fn missing_credential_leaves_legacy_checkpoint_byte_identical() {
    let fixture = Fixture::new();
    let (session_id, path) = fixture.interrupted_checkpoint();
    let before = std::fs::read(&path).unwrap();

    let output = fixture
        .command(&["pipeline", "resume", &session_id])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{}", text(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Authentication failed"),
        "the credential check must be the refusal: {}",
        text(&output)
    );
    assert_eq!(
        String::from_utf8_lossy(&std::fs::read(&path).unwrap()),
        String::from_utf8_lossy(&before),
        "a failed precondition must leave the saved checkpoint byte-identical"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("Resumed session"),
        "{}",
        text(&output)
    );
    assert!(!path.with_file_name("checkpoint.tmp").exists());
}

#[test]
fn passing_checks_mark_running_before_the_first_provider_request() {
    let fixture = Fixture::new();
    let (session_id, path) = fixture.interrupted_checkpoint();
    let before: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(before["status"], "Interrupted");

    // Every outbound request (provider endpoint or proxied traffic) lands on
    // this local listener. The status is read when the first one arrives.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let seen_path = path.clone();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let status = status_of(&seen_path);
            let mut head = [0_u8; 256];
            let read = stream.read(&mut head).unwrap_or(0);
            let _ = tx.send((status, String::from_utf8_lossy(&head[..read]).to_string()));
            // Hold the connection open so the run blocks until it is killed.
            std::thread::park();
        }
    });

    let mut child = fixture
        .command(&["pipeline", "resume", &session_id])
        .env("ANTHROPIC_API_KEY", "test-key-local-listener")
        .env("ANTHROPIC_BASE_URL", &endpoint)
        .env("HTTP_PROXY", &endpoint)
        .env("HTTPS_PROXY", &endpoint)
        .env("ALL_PROXY", &endpoint)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let first_request = loop {
        if let Ok(seen) = rx.recv_timeout(Duration::from_millis(50)) {
            break Some(seen);
        }
        if child.try_wait().unwrap().is_some() {
            break rx.try_recv().ok();
        }
    };
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();

    let (status_at_request, request) =
        first_request.unwrap_or_else(|| panic!("no provider request: {}", text(&output)));
    assert_eq!(status_at_request, "Running", "request: {request}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Resumed session"),
        "{}",
        text(&output)
    );
    let mut after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(after["status"], "Running");
    assert_ne!(after["updated_at"], before["updated_at"]);
    let mut before = before;
    for value in [&mut before, &mut after] {
        let object = value.as_object_mut().unwrap();
        object.remove("status");
        object.remove("updated_at");
    }
    assert_eq!(after, before, "only status and updated_at may change");
}
