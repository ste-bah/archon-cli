use super::*;

#[cfg(unix)]
#[test]
fn evidence_423_c_real_shell_set_cr_is_setup_error_not_preimplementation_proof() {
    let command = "set -e\r\nprintf criterion-ran\n";
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .output()
        .expect("the classifier's target shell must run the check");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let result = run(output.status.code(), &stdout, &stderr);
    let classification = no_verdict(command, &result, &host());
    eprintln!(
        "EVIDENCE #423 c BEFORE: raw_script_bytes={command:?} AFTER: shell_exit={:?} stdout={stdout:?} stderr={stderr:?} classifier={} ",
        output.status.code(),
        classification.as_deref().unwrap_or("criterion_verdict")
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(!stderr.is_empty());
    assert!(!stdout.contains("criterion-ran"));
    assert!(
        classification
            .as_deref()
            .is_some_and(|reason| reason.contains("before any criterion could run"))
    );
}

#[cfg(unix)]
#[test]
fn evidence_423_d_clean_setup_then_criterion_failure_remains_a_verdict() {
    let command = "set -e\nfalse\n";
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .output()
        .expect("the classifier's target shell must run the check");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let result = run(output.status.code(), &stdout, &stderr);
    let classification = no_verdict(command, &result, &host());
    eprintln!(
        "EVIDENCE #423 d BEFORE: raw_script_bytes={command:?} AFTER: shell_exit={:?} stdout={stdout:?} stderr={stderr:?} classifier={} ",
        output.status.code(),
        classification.as_deref().unwrap_or("criterion_verdict")
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr.is_empty());
    assert_eq!(classification, None);
}
