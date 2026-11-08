use super::*;

fn mounts() -> MountSeals {
    MountSeals {
        seals: ["/first", "/second"]
            .map(|name| Seal {
                path: CString::new(name).unwrap(),
                display: name.into(),
                readonly: true,
                directory: false,
                submounts: false,
            })
            .into(),
        uid: Vec::new(),
        gid: Vec::new(),
    }
}

#[test]
fn bind_failure_stops_sealing_but_attempts_all_cleanup_steps() {
    let mounts = mounts();
    let mut visited = Vec::new();
    let failure = mounts
        .apply_with(true, |step, _| {
            visited.push(step);
            if step == "bind" {
                unsafe {
                    *libc::__errno_location() = libc::EACCES;
                }
                -1
            } else {
                0
            }
        })
        .unwrap_err();
    assert_eq!(failure.step, "bind");
    assert_eq!(failure.errno, libc::EACCES);
    assert_eq!(visited.iter().filter(|step| **step == "bind").count(), 1);
    assert_eq!(&visited[visited.len() - 4..], &STEPS[10..]);
    assert_eq!(visited.last(), Some(&"re-enter working directory"));
}

#[test]
fn failed_reentry_preserves_first_failure_and_reports_inherited_cwd() {
    let mounts = mounts();
    let mut cwd_reentered = false;
    let mut binds = 0;
    let failure = mounts
        .apply_with_status(true, &mut cwd_reentered, |step, _| {
            if step == "bind" {
                binds += 1;
            }
            if (step == "bind" && binds == 2) || step == "re-enter working directory" {
                unsafe {
                    *libc::__errno_location() = libc::EACCES;
                }
                -1
            } else {
                0
            }
        })
        .unwrap_err();
    assert_eq!(failure.step, "bind");
    assert_eq!(failure.seal, 1);
    assert!(!cwd_reentered);
    let report = mounts.outcome([
        step_index(failure.step),
        failure.errno,
        failure.seal as i32,
        0,
    ]);
    assert!(
        report.contains("applied seals: [read-only /first]"),
        "{report}"
    );
    assert!(
        report.contains("applied seals bypassable through the inherited working directory"),
        "{report}"
    );
    assert!(
        !mounts
            .outcome([step_index("bind"), libc::EACCES, 1, 1])
            .contains("inherited working directory")
    );
}
