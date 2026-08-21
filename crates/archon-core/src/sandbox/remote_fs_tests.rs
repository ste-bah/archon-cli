use std::sync::Mutex;

use super::scripts::shell_quote;
use super::*;

/// A transport that answers from a script rather than a host.
///
/// Every command this module builds is exercised through it, so the
/// construction and the parsing are both under test without a far side. What
/// it cannot test is the transport itself; that is the ssh/openshell half.
/// `Clone` shares the recorded calls rather than copying them: a re-rooted
/// filesystem clones its transport, and a double whose clone forgot what it had
/// been asked would make those assertions look like nothing happened.
#[derive(Debug, Clone)]
struct FakeExec {
    calls: std::sync::Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    reply: std::sync::Arc<Mutex<Vec<RemoteOutput>>>,
}

impl FakeExec {
    fn with(outputs: Vec<RemoteOutput>) -> Self {
        Self {
            calls: std::sync::Arc::new(Mutex::new(Vec::new())),
            reply: std::sync::Arc::new(Mutex::new(outputs)),
        }
    }

    fn ok(stdout: impl Into<Vec<u8>>) -> Self {
        Self::with(vec![RemoteOutput {
            status: Some(0),
            stdout: stdout.into(),
            stderr: Vec::new(),
        }])
    }

    fn last_script(&self) -> String {
        self.calls.lock().unwrap().last().unwrap().0.clone()
    }

    fn last_stdin(&self) -> Vec<u8> {
        self.calls.lock().unwrap().last().unwrap().1.clone()
    }
}

#[async_trait::async_trait]
impl RemoteExec for FakeExec {
    async fn run(&self, script: &str, stdin: &[u8]) -> io::Result<RemoteOutput> {
        self.calls
            .lock()
            .unwrap()
            .push((script.to_string(), stdin.to_vec()));
        let mut reply = self.reply.lock().unwrap();
        if reply.len() > 1 {
            Ok(reply.remove(0))
        } else {
            Ok(reply[0].clone())
        }
    }

    fn label(&self) -> &'static str {
        "fake sandbox"
    }
}

fn fs_for(exec: FakeExec) -> RemoteFs<FakeExec> {
    RemoteFs::new(exec, WorkspaceMap::new("/host/proj", "/srv/ws"))
}

const NASTY: &str = "/srv/ws/a b/it's; rm -rf / #$(id)`whoami`\n.txt";

#[test]
fn quoting_survives_every_metacharacter_that_could_end_the_word() {
    let quoted = shell_quote(NASTY);

    assert!(quoted.starts_with('\''));
    assert!(quoted.ends_with('\''));
    // The only way out of a single-quoted string is a quote, and each one is
    // closed, escaped and reopened rather than emitted bare.
    assert!(quoted.contains(r#"it'\''s"#));
}

/// The quoting helper judged by the only authority that matters: a shell.
///
/// Asserting on the shape of the escaped string only proves it matches what
/// this file expects. Handing it to `/bin/sh` and asking what argument came
/// out the other side proves the property the helper exists for.
#[cfg(unix)]
#[test]
fn a_real_shell_hands_the_path_back_unchanged() {
    for value in [
        NASTY,
        "/plain/path",
        "/with space/x",
        "'",
        "''",
        "$HOME",
        "a\\b",
        "-rf",
    ] {
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", shell_quote(value)))
            .output()
            .expect("/bin/sh");

        assert!(output.status.success(), "sh rejected {value:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), value);
    }
}

#[test]
fn a_path_full_of_shell_syntax_stays_one_argument() {
    let script = read_script(NASTY);
    let tail = script.split("base64 < ").nth(1).unwrap();

    // Nothing of ours follows the path but the newline: the whole hostile
    // string is inside the quotes, so none of it can become a command.
    assert_eq!(tail, format!("{}\n", shell_quote(NASTY)));
}

#[test]
fn every_path_taking_script_quotes_its_path() {
    let quoted = shell_quote(NASTY);
    for script in [
        read_script(NASTY),
        write_script(NASTY),
        create_dir_all_script(NASTY),
        metadata_script(NASTY),
        read_dir_script(NASTY),
        remove_file_script(NASTY),
        rename_script(NASTY, "/srv/ws/plain"),
    ] {
        assert!(script.contains(&quoted), "unquoted path in: {script}");
        // Blank out the quoted occurrences; whatever is left is the script's
        // own text, and none of the path's syntax may have leaked into it.
        let skeleton = script.replace(&quoted, "<PATH>");
        assert!(!skeleton.contains("rm -rf"), "path leaked into: {script}");
        assert!(!skeleton.contains("$(id)"), "path leaked into: {script}");
        assert!(!skeleton.contains('`'), "path leaked into: {script}");
    }
}

#[test]
fn write_script_confirms_the_size_and_cleans_up_its_temp_file() {
    let script = write_script("/srv/ws/main.rs");

    assert!(script.contains("base64 -d"));
    assert!(script.contains("base64 -D"), "no BSD base64 fallback");
    assert!(script.contains("mv -f -- \"$tmp\" '/srv/ws/main.rs'"));
    assert!(script.contains("rm -f -- \"$tmp\""));
    assert!(script.contains("wc -c < '/srv/ws/main.rs'"));
}

#[test]
fn missing_base64_on_the_far_side_is_declared_not_assumed() {
    assert!(read_script("/a").contains("command -v base64"));
    assert!(read_script("/a").contains("exit 97"));
    // stat needs no base64, so it does not demand one.
    assert!(!metadata_script("/a").contains("command -v base64"));
}

#[test]
fn remove_uses_plain_rm_so_a_missing_file_still_fails() {
    assert!(!remove_file_script("/a").contains("rm -f"));
}

#[test]
fn glob_patterns_that_the_shell_would_interpret_are_refused() {
    for pattern in [
        "$(id)",
        "`whoami`",
        "a; rm -rf /",
        "a b",
        "a|b",
        "a>b",
        "a\\b",
        "\"a\"",
        "'a'",
        "~/secrets",
        "a\nb",
        "",
    ] {
        assert!(
            validate_glob_pattern(pattern).is_err(),
            "accepted dangerous pattern {pattern:?}"
        );
    }
}

#[test]
fn ordinary_glob_patterns_are_accepted() {
    for pattern in [
        "*.rs",
        "**/*.rs",
        "src/**/mod.rs",
        "[a-z]*.toml",
        "a{b,c}.md",
    ] {
        validate_glob_pattern(pattern).unwrap();
    }
}

#[test]
fn globstar_is_required_only_when_the_pattern_uses_it() {
    assert!(glob_script("/srv/ws", "**/*.rs").contains("shopt -s globstar"));
    assert!(!glob_script("/srv/ws", "*.rs").contains("shopt -s globstar"));
    assert!(glob_script("/srv/ws", "*.rs").contains("shopt -s nullglob"));
    assert!(glob_script("/srv/ws", "*.rs").contains("cd -- '/srv/ws'"));
}

#[test]
fn workspace_paths_are_translated_and_remote_paths_pass_through() {
    let map = WorkspaceMap::new("/host/proj", "/srv/ws/");

    assert_eq!(
        map.to_remote(Path::new("/host/proj/src/main.rs")).unwrap(),
        "/srv/ws/src/main.rs"
    );
    assert_eq!(map.to_remote(Path::new("/host/proj")).unwrap(), "/srv/ws");
    assert_eq!(
        map.to_remote(Path::new("/tmp/scratch")).unwrap(),
        "/tmp/scratch"
    );
}

#[test]
fn a_path_outside_the_workspace_is_refused_rather_than_guessed() {
    let map = WorkspaceMap::new("/host/proj", "/srv/ws");

    assert!(map.to_remote(Path::new("relative/path")).is_err());
    assert!(
        map.to_remote(Path::new("/host/proj/../etc/shadow"))
            .is_err()
    );
}

#[test]
fn stat_output_is_read_from_both_dialects() {
    let gnu = parse_stat("4096 1750000000 regular file\n").unwrap();
    assert_eq!(gnu.len, 4096);
    assert_eq!(gnu.modified_nanos, Some(1_750_000_000_000_000_000));
    assert!(!gnu.is_dir);

    let bsd = parse_stat("128 1750000001 Directory\n").unwrap();
    assert_eq!(bsd.len, 128);
    assert!(bsd.is_dir);

    assert!(parse_stat("stat: cannot stat").is_none());
    assert!(parse_stat("").is_none());
}

#[tokio::test]
async fn read_decodes_the_payload_the_far_side_encoded() {
    let binary: Vec<u8> = (0u8..=255).collect();
    let fs = fs_for(FakeExec::ok(BASE64.encode(&binary)));

    let got = fs.read(Path::new("/host/proj/blob.bin")).await.unwrap();

    assert_eq!(got, binary);
    assert!(
        fs.exec
            .last_script()
            .contains("base64 < '/srv/ws/blob.bin'")
    );
}

#[tokio::test]
async fn a_chatty_login_profile_fails_the_read_instead_of_corrupting_it() {
    let fs = fs_for(FakeExec::ok("Welcome to the box!\nYou have mail.\n"));

    let error = fs.read(Path::new("/host/proj/a.txt")).await.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("shell profile"));
}

#[tokio::test]
async fn a_missing_file_reports_not_found() {
    let fs = fs_for(FakeExec::with(vec![RemoteOutput {
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"bash: line 3: /srv/ws/gone.txt: No such file or directory\n".to_vec(),
    }]));

    let error = fs.read(Path::new("/host/proj/gone.txt")).await.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[tokio::test]
async fn an_absent_base64_is_reported_as_unsupported_not_as_an_empty_file() {
    let fs = fs_for(FakeExec::with(vec![RemoteOutput {
        status: Some(97),
        stdout: Vec::new(),
        stderr: b"archon-fs: base64 is not available in the sandbox world\n".to_vec(),
    }]));

    let error = fs.read(Path::new("/host/proj/a.txt")).await.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert!(error.to_string().contains("base64"));
}

#[tokio::test]
async fn write_sends_base64_on_stdin_and_accepts_the_confirmed_size() {
    let contents = b"fn main() {}\n\x00\xff";
    let fs = fs_for(FakeExec::ok(format!("{}\n", contents.len())));

    fs.write(Path::new("/host/proj/main.rs"), contents)
        .await
        .unwrap();

    assert_eq!(fs.exec.last_stdin(), BASE64.encode(contents).into_bytes());
    assert!(fs.exec.last_script().contains("'/srv/ws/main.rs'"));
}

#[tokio::test]
async fn a_transport_that_swallows_stdin_is_an_error_not_a_success() {
    // The far side ran, exited 0, and left a zero-byte file. Reporting Ok here
    // is exactly the fabricated success this seam exists to prevent.
    let fs = fs_for(FakeExec::ok("0\n"));

    let error = fs
        .write(Path::new("/host/proj/main.rs"), b"twelve bytes")
        .await
        .unwrap_err();

    assert!(error.to_string().contains("did not deliver the whole file"));
}

#[tokio::test]
async fn a_write_the_far_side_never_confirmed_is_an_error() {
    let fs = fs_for(FakeExec::ok(""));

    let error = fs
        .write(Path::new("/host/proj/main.rs"), b"x")
        .await
        .unwrap_err();

    assert!(error.to_string().contains("could not be confirmed"));
}

#[tokio::test]
async fn read_dir_splits_on_nul_so_newlines_in_names_survive() {
    let listing = b"/srv/ws/a.rs\0/srv/ws/od\nd name\0".to_vec();
    let fs = fs_for(FakeExec::ok(BASE64.encode(&listing)));

    let entries = fs.read_dir(Path::new("/host/proj")).await.unwrap();

    assert_eq!(
        entries,
        vec![
            PathBuf::from("/srv/ws/a.rs"),
            PathBuf::from("/srv/ws/od\nd name"),
        ]
    );
}

#[tokio::test]
async fn glob_returns_paths_rooted_in_the_far_sides_workspace() {
    let fs = fs_for(FakeExec::ok(BASE64.encode(b"src/main.rs\0src/lib.rs\0")));

    let matched = fs
        .glob(Path::new("/host/proj"), "src/**/*.rs")
        .await
        .unwrap();

    assert_eq!(
        matched,
        vec![
            PathBuf::from("/srv/ws/src/main.rs"),
            PathBuf::from("/srv/ws/src/lib.rs"),
        ]
    );
}

#[tokio::test]
async fn glob_refuses_a_hostile_pattern_before_it_reaches_the_transport() {
    let fs = fs_for(FakeExec::ok(""));

    let error = fs
        .glob(Path::new("/host/proj"), "$(touch /tmp/pwned)")
        .await
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(fs.exec.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn metadata_and_version_come_from_the_world_that_holds_the_file() {
    let fs = fs_for(FakeExec::ok("17 1750000000 regular file\n"));

    let meta = fs.metadata(Path::new("/host/proj/a.txt")).await.unwrap();

    assert_eq!(meta.len, 17);
    assert_eq!(meta.modified_nanos, Some(1_750_000_000_000_000_000));
    assert!(fs.version(Path::new("/host/proj/a.txt")).await.is_some());
}

#[tokio::test]
async fn rename_names_both_sides_in_the_far_sides_terms() {
    let fs = fs_for(FakeExec::ok(""));

    fs.rename(Path::new("/host/proj/a.txt"), Path::new("/host/proj/b.txt"))
        .await
        .unwrap();

    assert_eq!(
        fs.exec.last_script(),
        "mv -f -- '/srv/ws/a.txt' '/srv/ws/b.txt'\n"
    );
}
