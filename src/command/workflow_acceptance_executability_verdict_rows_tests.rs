#![allow(non_snake_case)]
use super::*;
use std::collections::BTreeMap;

#[test]
fn P1_26_baseline_build_failure_keeps_rule_four() {
    let why = no_verdict(
        "cargo test --lib x; true",
        &run(Some(101), "", RUSTC),
        &host(),
    )
    .unwrap();
    assert!(why.contains("did not build"));
}
#[test]
fn P1_33_reported_missing_program_in_check_order() {
    let at = Context::at(BTreeMap::from([("PATH".into(), String::new())]), vec![]);
    for error in [
        "p1_absent: command not found",
        "sh: p1_absent: not found",
        "sh: p1_absent: No such file or directory",
    ] {
        let why = no_verdict("p1_absent", &run(Some(127), "", error), &at).unwrap();
        assert!(why.contains("starts `p1_absent`"));
    }
}
#[test]
fn P1_34_guarded_missing_program_is_not_the_reported_program() {
    let at = Context::at(BTreeMap::from([("PATH".into(), String::new())]), vec![]);
    for guard in [
        "command -v earlier >/dev/null",
        "which earlier >/dev/null",
        "type earlier >/dev/null",
    ] {
        let command = format!("if {guard}; then earlier; fi; later");
        let why = no_verdict(&command, &run(Some(127), "", "sh: later: not found"), &at).unwrap();
        assert!(why.contains("starts `later`"), "{why}");
        assert!(!why.contains("starts `earlier`"));
    }
}
#[test]
fn P1_35_absent_tool_without_diagnostic_is_no_verdict() {
    let at = Context::at(BTreeMap::from([("PATH".into(), String::new())]), vec![]);
    assert!(no_verdict("missing; exit 127", &run(Some(127), "", ""), &at).is_some());
}
#[test]
#[cfg(unix)]
fn P1_36_permission_denied_names_program_and_host_cause() {
    let mut at = Context::at(BTreeMap::from([("PATH".into(), String::new())]), vec![]);
    at.host_path = None;
    let why = super::super::no_verdict_cause(
        "p1_denied",
        &run(
            Some(126),
            "",
            "/bin/sh: /full/path/p1_denied: Permission denied",
        ),
        &at,
    )
    .unwrap();
    assert!(why.0.contains("starts `p1_denied`"));
    assert_eq!(why.1, super::super::UnprovenCause::ProgramAbsent);

    let mut at = host();
    at.path = Some(String::new());
    let why =
        super::super::no_verdict_cause("sh", &run(Some(126), "", "sh: sh: Permission denied"), &at)
            .unwrap();
    assert_eq!(why.1, super::super::UnprovenCause::SiteLacks);
}
#[cfg(unix)]
fn denied_program(absolute: bool) {
    let bin = tempfile::tempdir().unwrap();
    let denied = bin.path().join("p1_denied");
    std::fs::write(&denied, "not executable").unwrap();
    let mut at = Context::at(
        BTreeMap::from([("PATH".into(), bin.path().display().to_string())]),
        vec![],
    );
    at.host_path = None;
    let program = if absolute {
        denied.to_str().unwrap()
    } else {
        "p1_denied"
    };
    let why = super::super::no_verdict_cause(
        program,
        &run(
            Some(126),
            "",
            &format!("/bin/sh: {}: Permission denied", denied.display()),
        ),
        &at,
    )
    .unwrap();
    assert!(why.0.contains(&format!("starts `{program}`")), "{why:?}");
    assert!(why.0.contains("not executable"), "{why:?}");
    assert!(
        why.0
            .contains("but cannot run it: the check must not depend on it"),
        "{why:?}"
    );
    assert!(!why.0.contains("make it executable"), "{why:?}");
    assert_eq!(why.1, super::super::UnprovenCause::SiteLacks);
}
#[test]
#[cfg(unix)]
fn P1_F2_a_bare_name_on_path_is_present_but_not_executable() {
    denied_program(false);
}
#[test]
#[cfg(unix)]
fn P1_F2_b_absolute_path_is_present_but_not_executable() {
    denied_program(true);
}
#[test]
fn P1_F2_c_missing_file_at_exit_127_is_program_absent() {
    let bin = tempfile::tempdir().unwrap();
    let absent = bin.path().join("p1_denied");
    let mut at = Context::at(
        BTreeMap::from([("PATH".into(), bin.path().display().to_string())]),
        vec![],
    );
    at.host_path = None;
    let programs = vec!["p1_denied"];
    #[cfg(unix)]
    let programs = [programs, vec![absent.to_str().unwrap()]].concat();
    #[cfg(not(unix))]
    let _ = absent;
    for program in programs {
        let why = super::super::no_verdict_cause(
            program,
            &run(Some(127), "", &format!("sh: {program}: not found")),
            &at,
        )
        .unwrap();
        assert_eq!(why.1, super::super::UnprovenCause::ProgramAbsent);
    }
}
#[test]
fn P1_37_missing_relative_deliverable_is_a_verdict() {
    let at = Context::at(BTreeMap::from([("PATH".into(), String::new())]), vec![]);
    assert!(no_verdict("./x.sh", &run(Some(127), "", "sh: ./x.sh: not found"), &at).is_none());
}

#[cfg(unix)]
fn denied_operand(command: &str, report: &str, wrappers: &[&str]) {
    let bin = tempfile::tempdir().unwrap();
    for wrapper in wrappers {
        std::fs::write(bin.path().join(wrapper), "available wrapper").unwrap();
    }
    let at = Context::at(
        BTreeMap::from([("PATH".into(), bin.path().display().to_string())]),
        vec![],
    );
    assert!(super::super::mutation::reported_permission_denied(
        report, "./tool"
    ));
    for wrapper in wrappers {
        assert!(!super::super::mutation::reported_permission_denied(
            report, wrapper
        ));
    }
    // A relative deliverable's mode is a check verdict, never a claim that
    // the site cannot execute the wrapper that actually reported the error.
    assert!(no_verdict(command, &run(Some(126), "", report), &at).is_none());
}
#[test]
#[cfg(unix)]
fn P1_D3_a_direct_denial_names_only_the_operand() {
    denied_operand("./tool", "sh: ./tool: Permission denied", &[]);
}
#[test]
#[cfg(unix)]
fn P1_D3_b_xargs_denial_does_not_blame_xargs() {
    denied_operand(
        "printf a | xargs ./tool",
        "xargs: ./tool: Permission denied",
        &["xargs"],
    );
}
#[test]
#[cfg(unix)]
fn P1_D3_c_gstdbuf_denial_does_not_blame_gstdbuf() {
    denied_operand(
        "gstdbuf -oL ./tool",
        "gstdbuf: failed to run command './tool': Permission denied",
        &["gstdbuf"],
    );
}
#[test]
#[cfg(unix)]
fn P1_D3_d_unnamed_denial_keeps_the_base_verdict() {
    let at = host();
    let report = "Permission denied";
    assert!(!super::super::mutation::reported_permission_denied(
        report, "sh"
    ));
    assert!(no_verdict("sh -c true", &run(Some(126), "", report), &at).is_none());
}
