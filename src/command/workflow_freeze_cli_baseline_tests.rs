//! Exercise production entries while their first slow operation is blocked.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The child keeps stdin open but empty (freeze), or stalls at the factory
/// boundary (set lint). A baseline moved past either operation cannot arrive.
pub(crate) fn assert_entry_baseline(test_name: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            test_name.split_once("::").unwrap().1,
            "--nocapture",
        ])
        .env("ARCHON_TEST_BLOCK_CLIENT_BUILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if crate::command::workflow_host_command_operational::reported_progress(line.as_bytes())
                == Some(0)
            {
                let _ = send.send(());
                break;
            }
        }
    });
    let baseline = receive.recv_timeout(Duration::from_secs(10));
    let _ = child.kill();
    let status = child.wait().unwrap();
    reader.join().unwrap();
    assert!(
        baseline.is_ok(),
        "production entry did not report progress before slow work: {status}"
    );
}

#[tokio::test]
async fn the_staged_freeze_reports_a_progress_baseline_before_it_builds_anything() {
    if std::env::var_os("ARCHON_TEST_BLOCK_CLIENT_BUILD").is_none() {
        assert_entry_baseline(concat!(
            module_path!(),
            "::the_staged_freeze_reports_a_progress_baseline_before_it_builds_anything"
        ));
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let config = archon_core::config::ArchonConfig::default();
    let env = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
    let action = crate::cli_args::WorkflowAction::FreezeAcceptance(
        crate::cli_args::WorkflowFreezeAcceptanceArgs {
            tasks: temp.path().join("tasks"),
            prd: temp.path().join("prd.md"),
            reauthor: Vec::new(),
            candidate_stdin: true,
            staging_root: Some(temp.path().join("staging")),
            gate_envelope: Some(temp.path().join("staging/envelope.json")),
            call_id: Some("baseline-test".into()),
        },
    );
    super::handle(&action, &config, &env, temp.path())
        .await
        .unwrap();
}
