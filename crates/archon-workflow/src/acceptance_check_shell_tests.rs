//! Issue 554: a check text the executing shell rejects, and a run whose own
//! shell reported a syntax error, are never a pass.

use super::*;
use crate::acceptance_check_crash::{CheckRunClass, classify_check_run};
use crate::acceptance_scratch::CheckResult;
use std::io::Write;

/// The R11 shape: `trap ... EXIT` runs first, then a `{ ... fi` block that
/// no POSIX shell parses. Under bash's POSIX mode this exits 0.
const TRAP_THEN_PARSE_ERROR: &str = "set -eu\nlog=$(mktemp)\ntrap 'rm -f \"$log\"' EXIT\nf() {\n grep -q x /dev/null || {\n  echo b\n fi\n}\n";

fn result(exit: Option<i32>, stderr: &str) -> CheckResult {
    CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: "AC-X".into(),
        exit_code: exit,
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        operational_error: None,
        environment_note: None,
    }
}

/// Run `command` exactly as a check site does: on the stdin of the
/// executing shell, with no environment.
#[cfg(unix)]
fn run(command: &str) -> CheckResult {
    let mut shell = archon_shell::spawn::command(archon_shell::resolve_posix_shell());
    let mut child =
        archon_shell::spawn::replace_environment(&mut shell, std::iter::empty::<(&str, &str)>())
            .arg("-s")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the executing shell starts");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(command.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let mut ran = result(
        output.status.code(),
        &String::from_utf8_lossy(&output.stderr),
    );
    ran.stdout = output.stdout;
    ran
}

#[cfg(unix)]
#[test]
fn the_executing_shell_rejects_a_trap_then_parse_error_text_with_its_line_554() {
    let error = shell_parse_error(TRAP_THEN_PARSE_ERROR).expect("the shell rejects it");
    assert!(
        error.contains("yntax error") && error.contains('7'),
        "{error}"
    );
    assert_eq!(
        shell_parse_error("set -eu\nif true; then echo ok; fi\n"),
        None
    );
    let finding = parse_finding("AC-X", &error);
    assert!(
        finding.contains("check 'AC-X'") && finding.contains(&error),
        "{finding}"
    );
}

/// A negative test of malformed input: a child shell rejects it on stdin,
/// and the check asserts exactly that. Its stderr carries the child's
/// syntax error, which no rule may read as the check's own.
const CHILD_REJECTS_INPUT: &str = "printf 'if then\\n' | sh -s; test $? -eq 2\n";

/// Parse-only mode never runs an alias definition: a block built from an
/// alias is outside the check contract, though it runs.
const ALIAS_BLOCK: &str = "alias begin='if true; then'\nbegin\ntest 1 -eq 1\nfi\n";

/// Codex r1: a valid check whose stderr holds a child shell's syntax error
/// is a pass. (Was `a_child_shells_syntax_error_and_an_alias_block_are_still_passes_554`.)
#[test]
fn a_child_shells_syntax_error_is_still_a_pass_554() {
    let mut ran = run(CHILD_REJECTS_INPUT);
    assert_eq!(ran.exit_code, Some(0), "{ran:?}");
    assert_eq!(
        classify_check_run(CHILD_REJECTS_INPUT, &ran),
        CheckRunClass::Passed
    );
    ran.classify_raw(CHILD_REJECTS_INPUT);
    assert!(ran.classification.as_ref().unwrap().passed);
    assert_eq!(shell_parse_error(CHILD_REJECTS_INPUT), None);
    // The R11 text fails the preflight: that is its one gate.
    assert!(shell_parse_error(TRAP_THEN_PARSE_ERROR).is_some());
}

/// The check contract: a text must parse under `-n`, whatever it defines
/// before. An alias-built block, a case pattern naming `alias`, and alias
/// text in a comment, a string or a here-doc are all rejected, with the
/// rule stated.
#[test]
fn an_alias_built_or_alias_naming_text_is_rejected_by_the_parse_rule_554() {
    let escaped = "\\alias begin='if true; then'\nbegin\ntest 1 -eq 1\nfi\n";
    let case = format!("case x in x|alias) true;; esac\n{TRAP_THEN_PARSE_ERROR}");
    let commented = format!("# check alias behavior\n{TRAP_THEN_PARSE_ERROR}");
    let quoted = format!("echo 'run alias begin first'\n{TRAP_THEN_PARSE_ERROR}");
    let heredoc = format!("cat <<'EOF'\nalias begin='if true; then'\nEOF\n{TRAP_THEN_PARSE_ERROR}");
    for text in [ALIAS_BLOCK, escaped, &case, &commented, &quoted, &heredoc] {
        let error = shell_parse_error(text).unwrap_or_else(|| panic!("fails open: {text}"));
        let finding = parse_finding("AC-X", &error);
        assert!(
            finding.contains(&error) && finding.contains("alias") && finding.contains("`-n`"),
            "{finding}"
        );
    }
}

/// Issue 554, stage `verify_command` (`sh -c`): a text its shell rejects is
/// never run and never accepted, whatever exit status a run would give.
#[cfg(unix)]
#[test]
fn a_verify_command_its_shell_cannot_parse_is_rejected_with_the_shell_error_554() {
    let root = tempfile::tempdir().unwrap();
    let error = crate::acceptance::run_verify_command(root.path(), Some(TRAP_THEN_PARSE_ERROR))
        .expect_err("a text the shell rejects is no pass");
    assert!(
        error.contains("does not parse") && error.contains("yntax error"),
        "{error}"
    );
}

/// Codex r1 for `verify_command`: a negative test of a nested shell is
/// accepted; an alias-built block is rejected by the parse rule. (Was
/// `a_verify_command_with_a_nested_shell_error_or_an_alias_block_is_accepted_554`.)
#[test]
fn a_verify_command_with_a_nested_shell_error_is_accepted_and_an_alias_block_rejected_554() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        crate::acceptance::run_verify_command(root.path(), Some("! sh -c 'if then'")),
        Ok(())
    );
    let error = crate::acceptance::run_verify_command(root.path(), Some(ALIAS_BLOCK))
        .expect_err("an alias-built block is outside the contract");
    assert!(
        error.contains("does not parse") && error.contains("alias"),
        "{error}"
    );
}

/// Codex r3 / security review: the preflight reads no file but the text.
/// A sourced broken file is judged by the run alone. (Was
/// `a_sourced_file_is_parsed_before_the_run_554`.)
#[test]
fn a_sourced_file_is_never_read_by_the_preflight_554() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("lib")).unwrap();
    std::fs::write(
        root.path().join("lib/broken.sh"),
        "f() {\n  true || {\n  fi\n}\n",
    )
    .unwrap();
    let text = ". ./lib/broken.sh\nf\n";
    assert_eq!(shell_parse_error(text), None);
    let verify = crate::acceptance::run_verify_command(root.path(), Some(". ./lib/broken.sh; f"))
        .expect_err("its run fails");
    assert!(!verify.contains("does not parse"), "{verify}");
    assert!(
        !verify.contains("lib/broken.sh") || !verify.contains("it sources"),
        "{verify}"
    );
}

#[test]
fn parse_cache_shares_exact_shell_and_text_results_567() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cache = ParseCache::default();
    let calls = AtomicUsize::new(0);
    let shell = archon_shell::resolve_posix_shell();
    for text in ["true\n", "if then\n"] {
        let expected = parse_with(shell.as_os_str(), text).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let (cache, calls, shell, expected) = (&cache, &calls, &shell, &expected);
                scope.spawn(move || {
                    assert_eq!(
                        &cached_parse(cache, shell.as_os_str(), text, || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            parse_with(shell.as_os_str(), text)
                        }),
                        expected
                    );
                });
            }
        });
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    for (shell, text) in [
        (OsStr::new("other-shell"), "true\n"),
        (shell.as_os_str(), "true"),
    ] {
        assert_eq!(
            cached_parse(&cache, shell, text, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Some("distinct key".into()))
            }),
            Some("distinct key".into())
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[test]
fn parse_cache_retries_spawn_wait_and_signal_failures_567() {
    use std::os::unix::process::ExitStatusExt;
    let missing = tempfile::tempdir().unwrap().path().join("missing-shell");
    for failure in [
        parse_with(missing.as_os_str(), "if then"),
        parse_output(Err(std::io::Error::other("wait failed"))),
        parse_output(Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(9),
            stdout: Vec::new(),
            stderr: b"interrupted, not a syntax diagnostic".to_vec(),
        })),
    ] {
        assert_eq!(failure, Err(()));
        let cache = ParseCache::default();
        let shell = OsStr::new("shell");
        assert_eq!(cached_parse(&cache, shell, "if then", || failure), None);
        assert!(cache.lock().unwrap().is_empty());
        let expected = shell_parse_error("if then").expect("syntax rejection on retry");
        assert_eq!(
            cached_parse(&cache, shell, "if then", || Ok(Some(expected.clone()))),
            Some(expected)
        );
        assert_eq!(cache.lock().unwrap().len(), 1);
    }
}

#[test]
fn parse_cache_is_bounded_and_unlocked_during_parsing_567() {
    let cache = ParseCache::default();
    for index in 0..=MAX_PARSE_CACHE_ENTRIES {
        assert_eq!(
            cached_parse(&cache, OsStr::new("shell"), &index.to_string(), || {
                assert!(cache.try_lock().is_ok(), "parser holds the cache lock");
                Ok(None)
            }),
            None
        );
        assert!(cache.lock().unwrap().len() <= MAX_PARSE_CACHE_ENTRIES);
        if index == MAX_PARSE_CACHE_ENTRIES {
            assert_eq!(
                cache.lock().unwrap().len(),
                1,
                "the cap clears the whole map"
            );
        }
    }
    assert!(
        !cache
            .lock()
            .unwrap()
            .contains_key(&(OsString::from("shell"), "0".into()))
    );
}

#[test]
fn waiting_caller_retries_its_own_probe_after_another_callers_failure_567() {
    let cache = ParseCache::default();
    let (started, ready) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let owner = scope.spawn(|| {
            cached_parse(&cache, OsStr::new("shell"), "text", || {
                started.send(()).unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    let joined = cache
                        .lock()
                        .unwrap()
                        .values()
                        .any(|pending| Arc::strong_count(pending) >= 3);
                    if joined {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "waiter never joined probe"
                    );
                    std::thread::yield_now();
                }
                Err(())
            })
        });
        ready.recv().unwrap();
        let waiter = scope.spawn(|| {
            cached_parse(&cache, OsStr::new("shell"), "text", || {
                Ok(Some("own syntax rejection".into()))
            })
        });
        assert_eq!(owner.join().unwrap(), None);
        assert_eq!(waiter.join().unwrap(), Some("own syntax rejection".into()));
    });
}

#[test]
fn cannot_execute_status_is_operational_and_not_cached_567() {
    use std::os::unix::process::ExitStatusExt;
    for code in [126, 127] {
        let cache = ParseCache::default();
        assert_eq!(
            cached_parse(&cache, OsStr::new("shell"), "text", || {
                parse_output(Ok(std::process::Output {
                    status: std::process::ExitStatus::from_raw(code << 8),
                    stdout: Vec::new(),
                    stderr: b"cannot execute".to_vec(),
                }))
            }),
            None
        );
        assert!(cache.lock().unwrap().is_empty());
        assert_eq!(
            cached_parse(&cache, OsStr::new("shell"), "text", || {
                Ok(Some("retried syntax rejection".into()))
            }),
            Some("retried syntax rejection".into())
        );
    }
}
