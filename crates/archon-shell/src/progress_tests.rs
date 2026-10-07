//! #356: activity lines reach stderr only in a child a supervisor reads.
use super::*;

/// Records activity once, as any observed check or provider event does.
#[test]
#[ignore = "internal child process"]
fn activity_child() {
    let progress = Progress::new(true);
    progress.record();
    progress.record();
}

fn child_stderr(supervised: Option<&str>) -> String {
    let mut command = crate::spawn::command(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "progress::tests::activity_child",
            "--ignored",
            "--nocapture",
        ])
        .env_remove(SUPERVISED_ENV)
        .stdin(std::process::Stdio::null());
    if let Some(value) = supervised {
        command.env(SUPERVISED_ENV, value);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn issue356_a_supervised_child_reports_activity_on_stderr_coalesced() {
    let stderr = child_stderr(Some("1"));
    assert_eq!(stderr.matches(ACTIVITY_LINE).count(), 1, "{stderr}");
}

#[test]
fn issue356_an_unsupervised_process_keeps_activity_off_stderr() {
    for supervised in [None, Some("0"), Some("")] {
        let stderr = child_stderr(supervised);
        assert!(!stderr.contains(ACTIVITY_LINE), "{supervised:?}: {stderr}");
    }
}
