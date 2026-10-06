use super::*;

fn session_only_remedy(count: usize, stale_identity: bool) {
    use archon_shell::process_tree::{Pinned, deliver, identity_of};
    use std::os::unix::process::CommandExt;
    struct Children(Vec<Pinned>);
    impl Drop for Children {
        fn drop(&mut self) {
            for &pin in &self.0 {
                deliver(pin, libc::SIGKILL);
            }
        }
    }
    let run = tempfile::tempdir().unwrap();
    let mut children = Children(Vec::new());
    let mut leader = archon_shell::spawn::command("python3");
    leader.args(["-c", "import os,subprocess,sys; children=[subprocess.Popen(['sleep','30'],preexec_fn=os.setpgrp,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL) for _ in range(int(sys.argv[1]))]; open('members','w').write(','.join(str(p.pid) for p in children))", &count.to_string()])
        .current_dir(run.path()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // SAFETY: only the async-signal-safe setsid syscall runs before exec.
    unsafe {
        leader.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut leader = leader.spawn().unwrap();
    let id = leader.id();
    assert!(leader.wait().unwrap().success());
    for pid in std::fs::read_to_string(run.path().join("members"))
        .unwrap()
        .split(',')
    {
        let pid = pid.parse().unwrap();
        children.0.push(Pinned {
            pid,
            start: identity_of(pid).unwrap().unwrap(),
        });
    }
    assert_eq!(
        group_running(id),
        Some(false),
        "original group must have ended"
    );
    let guard = record_group(
        &run.path().join(GROUP_RECORDS_DIR),
        id,
        id,
        Some(id),
        None,
        "cmd",
    )
    .unwrap();
    let stale = [(ended_group(), 1)];
    guard.keep(Some(if stale_identity { &stale } else { &[] }));
    let note = require_no_running_groups(run.path(), "run")
        .unwrap_err()
        .to_string();
    assert!(
        !note.contains(&format!("kill -TERM -{id}")),
        "ended group remedy: {note}"
    );
    for pin in &children.0 {
        assert!(note.contains(&format!("kill -TERM {}", pin.pid)), "{note}");
        assert!(note.contains(&pin.start.to_string()), "{note}");
    }
}
#[test]
fn session_member_in_another_group_has_a_working_remedy() {
    session_only_remedy(1, false);
}
#[test]
fn multiple_session_groups_have_working_remedies() {
    session_only_remedy(2, false);
}
#[test]
fn stale_recorded_identity_does_not_hide_session_remedy() {
    session_only_remedy(1, true);
}
