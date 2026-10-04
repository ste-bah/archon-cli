//! Issue 270: a check's teardown is verified for every process it started,
//! not only for the members of its own process group.
use super::*;
use crate::acceptance_world::AuthorizedCommand;
use crate::task_set_contract::TrustedCwd;

/// A site whose cwd is `root`; with `scratch`, `root` is also the audited
/// scratch root, as the scratch observer lays it out.
fn site(root: &Path, scratch: bool, timeout_secs: u64) -> CommandSite<'_> {
    CommandSite {
        project: root,
        repository: root,
        environment: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
        audit_root: scratch.then_some(root),
        audit_target: None,
        scratch_bytes: u64::MAX,
        output_bytes: 4096,
        timeout_secs,
        redactor: None,
    }
}

/// A descendant that leaves the check's process group (`how` is the Perl
/// statement that does it), records its pid, then writes `late` after
/// `delay` seconds unless it is killed first.
struct Escaper {
    pid: PathBuf,
    late: PathBuf,
}

impl Escaper {
    fn new(dir: &Path) -> Self {
        Self {
            pid: dir.join("escaper-pid"),
            late: dir.join("escaper-late"),
        }
    }

    fn start(&self, how: &str, delay: u32) -> String {
        self.start_with(how, delay, "")
    }

    /// As [`Self::start`], with the descendant's stdio away from the check's
    /// pipes, as a daemon leaves them.
    fn start_quiet(&self, how: &str, delay: u32) -> String {
        self.start_with(how, delay, " </dev/null >/dev/null 2>&1")
    }

    fn start_with(&self, how: &str, delay: u32, redirect: &str) -> String {
        format!(
            "perl -MPOSIX -e '{how}; open(F, \">\", $ARGV[0]); print F $$; close F; sleep {delay}; open(F, \">\", $ARGV[1]); close F' '{pid}' '{late}'{redirect} &\nuntil [ -s '{pid}' ]; do sleep 0.01; done",
            pid = self.pid.display(),
            late = self.late.display(),
        )
    }

    fn pid(&self) -> i32 {
        std::fs::read_to_string(&self.pid)
            .expect("descendant never started")
            .trim()
            .parse()
            .unwrap()
    }

    async fn assert_killed(&self, delay: u32, cause: &str) {
        let _kill = Kill(self.pid());
        tokio::time::sleep(Duration::from_millis(u64::from(delay) * 1000 + 700)).await;
        assert!(!self.late.exists(), "descendant outlived {cause}");
    }
}

/// Kills `pid` when the test ends, however it ends.
struct Kill(i32);

impl Drop for Kill {
    fn drop(&mut self) {
        // SAFETY: signals only the process this test started.
        unsafe {
            libc::kill(self.0, libc::SIGKILL);
        }
    }
}

const OWN_GROUP: &str = "setpgid(0, 0)";
const OWN_SESSION: &str = "POSIX::setsid()";

async fn run(site: &CommandSite<'_>, text: &str) -> CheckResult {
    let command = AuthorizedCommand::for_test(text, TrustedCwd::ProjectRoot);
    run_at(site, "AC-1", &command, Arc::new(AtomicBool::new(false)))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_timed_out_check_kills_a_setsid_descendant_whose_parent_still_runs() {
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    let text = format!("{}\nsleep 30", escaper.start(OWN_SESSION, 2));
    let result = run(&site(temp.path(), false, 1), &text).await;
    assert_eq!(
        result.operational_error.as_deref(),
        Some(CHECK_TIMED_OUT),
        "{result:?}"
    );
    escaper.assert_killed(2, "the check's timeout").await;
}

#[tokio::test]
async fn a_cancelled_check_kills_a_descendant_in_its_own_process_group() {
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    // Cancellation is seen on the first tick, after the descendant started.
    let text = format!("{}\nsleep 30", escaper.start(OWN_GROUP, 2));
    let command = AuthorizedCommand::for_test(&text, TrustedCwd::ProjectRoot);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let pid_file = escaper.pid.clone();
    tokio::spawn(async move {
        while !pid_file.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        flag.store(true, Ordering::SeqCst);
    });
    let site = site(temp.path(), false, 30);
    let result = run_at(&site, "AC-1", &command, cancel).await.unwrap();
    assert!(
        result
            .operational_error
            .as_deref()
            .is_some_and(|e| e.contains("cancellation")),
        "{result:?}"
    );
    escaper.assert_killed(2, "cancellation").await;
}

#[tokio::test]
async fn a_detached_descendant_holding_the_output_pipes_is_an_operational_error() {
    // Detached the same way, but still writing to the check's pipes: they
    // never reach end of file, and that is the evidence of the escape.
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    let text = format!("{}\nexit 0", escaper.start(OWN_SESSION, 30));
    let result = run(&site(temp.path(), false, 20), &text).await;
    let _kill = Kill(escaper.pid());
    assert!(
        result
            .operational_error
            .as_deref()
            .is_some_and(|e| e.contains("pipes stayed open")),
        "{result:?}"
    );
}

#[tokio::test]
async fn files_this_process_holds_in_the_scratch_are_not_a_detached_descendant() {
    let temp = tempfile::tempdir().unwrap();
    let _held = std::fs::File::create(temp.path().join("held-by-the-observer")).unwrap();
    let result = run(&site(temp.path(), true, 20), "echo ok").await;
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    assert!(result.operational_error.is_none(), "{result:?}");
}

#[tokio::test]
async fn a_descendant_that_exits_with_its_check_leaves_a_clean_teardown() {
    let temp = tempfile::tempdir().unwrap();
    let text = "perl -MPOSIX -e 'POSIX::setsid(); exit 0' & wait\necho ok";
    let result = run(&site(temp.path(), true, 20), text).await;
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    assert!(result.operational_error.is_none(), "{result:?}");
}

/// Waits, within a bound, for `pid` to leave the process table (an orphan is
/// reaped by init at once), and kills it if it does not.
async fn assert_gone(pid: i32, cause: &str) {
    let start = std::time::Instant::now();
    // SAFETY: signal 0 only probes the process this test started.
    while unsafe { libc::kill(pid, 0) } == 0 {
        if start.elapsed() > Duration::from_secs(3) {
            drop(Kill(pid));
            panic!("descendant {pid} outlived {cause}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_direct_check_kills_a_setsid_descendant_seen_while_it_ran() {
    // Round 2, rule 2: the descendant left the group and the session, and its
    // parent (the check's shell) exits before teardown. A scan while it ran
    // saw it, so teardown must still reach it.
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    let text = format!(
        "{}\nsleep 1.2\nexit 0",
        escaper.start_quiet(OWN_SESSION, 30)
    );
    let result = run(&site(temp.path(), false, 20), &text).await;
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    assert_gone(escaper.pid(), "a direct check's teardown").await;
}

#[tokio::test]
async fn an_unrelated_reader_of_the_scratch_does_not_fail_the_check() {
    // Round 2, rule 5: a process this check never started (an editor, an
    // indexer) reads a file in the scratch. It is not the check's escape.
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("read-by-another");
    std::fs::write(&file, "x").unwrap();
    let ready = temp.path().join("reader-ready");
    let mut reader = std::process::Command::new("perl")
        .args([
            "-e",
            "open(F, '<', $ARGV[0]) or die; open(R, '>', $ARGV[1]); close R; sleep 30",
        ])
        .arg(&file)
        .arg(&ready)
        .current_dir("/")
        .spawn()
        .unwrap();
    while !ready.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let result = run(&site(temp.path(), true, 20), "echo ok").await;
    let _ = reader.kill();
    let _ = reader.wait();
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    assert!(result.operational_error.is_none(), "{result:?}");
}

#[tokio::test]
async fn a_never_seen_writer_left_in_the_scratch_is_an_operational_error() {
    // Detached at once, so no scan ever saw it; it still holds a file in the
    // scratch open for writing, which no unrelated reader does.
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    let how = format!(
        "POSIX::setsid(); open(W, \">>\", \"{}\")",
        temp.path().join("written").display()
    );
    let text = format!("{}\nexit 0", escaper.start_quiet(&how, 30));
    let result = run(&site(temp.path(), true, 20), &text).await;
    let _kill = Kill(escaper.pid());
    assert!(
        result
            .operational_error
            .as_deref()
            .is_some_and(|e| e.contains(&escaper.pid().to_string())),
        "{result:?}"
    );
}

#[tokio::test]
async fn abort_before_first_scan_kills_an_original_group_descendant() {
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    let text = format!("{}\nsleep 30", escaper.start_quiet("", 30));
    let root = temp.path().to_path_buf();
    let task = tokio::spawn(async move {
        let command = AuthorizedCommand::for_test(&text, TrustedCwd::ProjectRoot);
        run_at(
            &site(&root, false, 30),
            "AC-1",
            &command,
            Arc::new(AtomicBool::new(false)),
        )
        .await
    });
    let start = std::time::Instant::now();
    while std::fs::metadata(&escaper.pid).map_or(true, |m| m.len() == 0) {
        assert!(start.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "test missed pre-scan window"
    );
    let pid = escaper.pid();
    let _kill = Kill(pid);
    task.abort();
    let _ = task.await;
    assert_gone(pid, "abort before first scan").await;
}
