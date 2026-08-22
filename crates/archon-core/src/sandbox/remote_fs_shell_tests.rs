//! The command layer against a real shell.
//!
//! There is no ssh host and no openshell gateway in CI, so the *transport*
//! cannot be exercised here. The commands can: the far side runs
//! `/bin/bash -lc <script>` with the payload on stdin, and so does this. What
//! these tests prove is that the scripts themselves — the quoting, the base64
//! round trip, `stat`, `find -print0`, `globstar`, the temp-file rename, and
//! the stdin plumbing in `run_transport_process` — do what they claim against
//! a genuine bash. What they cannot prove is that ssh and openshell carry
//! stdin and stdout through unchanged.

use std::path::PathBuf;

use tempfile::TempDir;

use super::*;

/// The far side, minus the wire.
#[derive(Debug, Clone)]
struct BashExec;

#[async_trait::async_trait]
impl RemoteExec for BashExec {
    async fn run(&self, script: &str, stdin: &[u8]) -> io::Result<RemoteOutput> {
        let mut cmd = TokioCommand::new("/bin/bash");
        cmd.arg("-lc").arg(script);
        run_transport_process(cmd, stdin, 30_000, "bash").await
    }

    fn label(&self) -> &'static str {
        "local bash"
    }
}

const HOST_ROOT: &str = "/host/proj";

fn world() -> (TempDir, RemoteFs<BashExec>) {
    let dir = tempfile::tempdir().unwrap();
    let remote_root = dir.path().to_str().unwrap().to_string();
    let fs = RemoteFs::new(BashExec, WorkspaceMap::new(HOST_ROOT, remote_root));
    (dir, fs)
}

fn host(relative: &str) -> PathBuf {
    PathBuf::from(format!("{HOST_ROOT}/{relative}"))
}

#[tokio::test]
async fn a_binary_file_round_trips_through_the_command_layer() {
    let (dir, fs) = world();
    let bytes: Vec<u8> = (0..=255u8).chain(0..=255u8).collect();

    fs.write(&host("blob.bin"), &bytes).await.unwrap();

    // The bytes really landed on disk, not just in an echoed byte count.
    assert_eq!(std::fs::read(dir.path().join("blob.bin")).unwrap(), bytes);
    assert_eq!(fs.read(&host("blob.bin")).await.unwrap(), bytes);
}

#[tokio::test]
async fn a_file_the_shell_wrote_is_the_file_the_read_returns() {
    let (dir, fs) = world();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let text = fs.read_to_string(&host("main.rs")).await.unwrap();

    assert_eq!(text, "fn main() {}\n");
}

#[tokio::test]
async fn a_filename_full_of_shell_syntax_is_written_and_read_back() {
    let (dir, fs) = world();
    let name = "it's a $(file) `here`;rm -rf .txt";

    fs.write(&host(name), b"safe").await.unwrap();

    assert_eq!(
        std::fs::read(dir.path().join(name)).unwrap(),
        b"safe".to_vec()
    );
    assert_eq!(fs.read(&host(name)).await.unwrap(), b"safe".to_vec());
}

#[tokio::test]
async fn an_empty_file_is_a_legitimate_write_not_a_dropped_payload() {
    let (dir, fs) = world();

    fs.write(&host("empty.txt"), b"").await.unwrap();

    assert_eq!(
        std::fs::read(dir.path().join("empty.txt")).unwrap(),
        Vec::<u8>::new()
    );
}

#[tokio::test]
async fn metadata_reports_the_real_size_and_a_real_mtime() {
    let (dir, fs) = world();
    std::fs::write(dir.path().join("a.txt"), "0123456789").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();

    let file = fs.metadata(&host("a.txt")).await.unwrap();
    let subdir = fs.metadata(&host("sub")).await.unwrap();

    assert_eq!(file.len, 10);
    assert!(!file.is_dir);
    assert!(subdir.is_dir);
    // Checked against the host's own view of the same file, not against the
    // wall clock. "Close to now" is a stopwatch standing in for a fact that can
    // be established directly, and it fails whenever the clock steps between
    // the write and the assertion — which under WSL it does, sporadically.
    // `stat -c %Y` reports whole seconds, so the comparison is at that
    // granularity.
    let host_seconds = std::fs::metadata(dir.path().join("a.txt"))
        .and_then(|meta| meta.modified())
        .expect("the host can see the file too")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a modification time after the epoch")
        .as_secs();
    let modified = file.modified_nanos.expect("no mtime from stat");
    assert_eq!(
        modified / 1_000_000_000,
        u128::from(host_seconds),
        "stat reported a different mtime than the host holds for the same file"
    );
    assert!(fs.version(&host("a.txt")).await.is_some());
}

#[tokio::test]
async fn a_missing_file_is_not_found_rather_than_empty() {
    let (_dir, fs) = world();

    let error = fs.read(&host("absent.txt")).await.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(fs.metadata(&host("absent.txt")).await.is_err());
    assert!(!fs.exists(&host("absent.txt")).await);
    assert!(fs.remove_file(&host("absent.txt")).await.is_err());
}

#[tokio::test]
async fn read_dir_lists_awkward_names_without_splitting_them() {
    let (dir, fs) = world();
    std::fs::write(dir.path().join("plain.txt"), "x").unwrap();
    std::fs::write(dir.path().join("two words.txt"), "x").unwrap();
    std::fs::write(dir.path().join("it's.txt"), "x").unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();

    let mut names: Vec<String> = fs
        .read_dir(Path::new(HOST_ROOT))
        .await
        .unwrap()
        .into_iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();

    assert_eq!(names, ["it's.txt", "nested", "plain.txt", "two words.txt"]);
}

/// `**` needs `globstar`, which arrived in bash 4. macOS still ships bash 3.2,
/// so both outcomes are correct behaviour and which one is right here depends
/// on the machine — asserting only the match made this a test of the runner's
/// bash rather than of the code. An old shell must produce the *documented*
/// refusal, naming globstar, rather than silently matching nothing: a glob that
/// quietly returned no files would read as "no such source files" and send the
/// agent looking for the wrong problem.
#[tokio::test]
async fn glob_descends_with_globstar_or_says_the_shell_is_too_old() {
    let (dir, fs) = world();
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("src/a.rs"), "x").unwrap();
    std::fs::write(dir.path().join("src/nested/b.rs"), "x").unwrap();
    std::fs::write(dir.path().join("src/notes.md"), "x").unwrap();

    let result = fs.glob(Path::new(HOST_ROOT), "**/*.rs").await;

    if local_bash_has_globstar().await {
        let mut matched: Vec<String> = result
            .expect("a bash with globstar must match")
            .into_iter()
            .map(|path| {
                path.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        matched.sort();

        assert_eq!(matched, ["src/a.rs", "src/nested/b.rs"]);
    } else {
        let error = result.expect_err("a bash without globstar cannot match **");

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(
            error.to_string().contains("globstar"),
            "the refusal has to name what is missing: {error}"
        );
    }
}

/// Asked of the same shell the scripts run in, not inferred from the platform.
async fn local_bash_has_globstar() -> bool {
    BashExec
        .run("shopt -s globstar 2>/dev/null && echo yes\n", &[])
        .await
        .is_ok_and(|out| out.status == Some(0) && out.stdout.starts_with(b"yes"))
}

#[tokio::test]
async fn a_glob_that_matches_nothing_is_empty_not_the_pattern_itself() {
    let (_dir, fs) = world();

    let matched = fs.glob(Path::new(HOST_ROOT), "*.nope").await.unwrap();

    assert!(matched.is_empty());
}

#[tokio::test]
async fn directories_are_created_moved_and_removed() {
    let (dir, fs) = world();

    fs.create_dir_all(&host("a/b/c")).await.unwrap();
    assert!(dir.path().join("a/b/c").is_dir());

    fs.write(&host("a/b/c/one.txt"), b"one").await.unwrap();
    fs.rename(&host("a/b/c/one.txt"), &host("a/two.txt"))
        .await
        .unwrap();
    assert!(!dir.path().join("a/b/c/one.txt").exists());
    assert_eq!(
        std::fs::read(dir.path().join("a/two.txt")).unwrap(),
        b"one".to_vec()
    );

    fs.remove_file(&host("a/two.txt")).await.unwrap();
    assert!(!dir.path().join("a/two.txt").exists());
}

#[tokio::test]
async fn a_write_leaves_no_temp_file_behind() {
    let (dir, fs) = world();

    fs.write(&host("x.txt"), b"content").await.unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["x.txt"]);
}
