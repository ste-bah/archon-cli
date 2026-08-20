//! Tests for docker workspace path translation (#201 Phase 2).
//!
//! The bytes are the host's — that is what the bind mount means — so what
//! needs proving is that a path the *container* named resolves to the same
//! file, and that a path the container could not have meant is refused.

use super::super::DockerConfig;
use super::*;

fn workspace() -> (tempfile::TempDir, DockerFs) {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs = DockerFs::new(dir.path());
    (dir, fs)
}

#[tokio::test]
async fn a_container_path_reads_the_host_file_it_names() {
    let (dir, fs) = workspace();
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").expect("write");

    let bytes = fs
        .read(Path::new("/workspace/src/main.rs"))
        .await
        .expect("the path Bash would have printed");

    assert_eq!(bytes, b"fn main() {}");
}

#[tokio::test]
async fn a_host_path_still_works_unchanged() {
    let (dir, fs) = workspace();
    let file = dir.path().join("plain.txt");
    std::fs::write(&file, "host bytes").expect("write");

    assert_eq!(
        fs.read(&file).await.expect("host path"),
        b"host bytes",
        "the common case — paths Read and Glob handed out — must not be mangled"
    );
}

#[tokio::test]
async fn writing_to_a_container_path_lands_on_the_host_file() {
    let (dir, fs) = workspace();

    fs.write(Path::new("/workspace/written.txt"), b"through the mount")
        .await
        .expect("write");

    assert_eq!(
        std::fs::read(dir.path().join("written.txt")).expect("host read"),
        b"through the mount"
    );
}

#[tokio::test]
async fn the_workspace_root_itself_translates() {
    let (dir, fs) = workspace();
    std::fs::write(dir.path().join("a.txt"), "x").expect("write");

    let meta = fs
        .metadata(Path::new("/workspace"))
        .await
        .expect("the mount point is the working directory");

    assert!(meta.is_dir);
    assert_eq!(
        fs.read_dir(Path::new("/workspace"))
            .await
            .expect("read_dir")
            .len(),
        1
    );
}

#[tokio::test]
async fn listings_come_back_as_container_paths() {
    let (dir, fs) = workspace();
    std::fs::write(dir.path().join("one.txt"), "1").expect("write");

    let entries = fs
        .read_dir(Path::new("/workspace"))
        .await
        .expect("read_dir");

    assert_eq!(
        entries,
        vec![PathBuf::from("/workspace/one.txt")],
        "a path handed back to the model must be one it can paste into Bash"
    );
    let _ = dir;
}

#[tokio::test]
async fn glob_results_come_back_as_container_paths() {
    let (dir, fs) = workspace();
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("src").join("lib.rs"), "x").expect("write");

    let matched = fs
        .glob(Path::new("/workspace"), "src/*.rs")
        .await
        .expect("glob");

    assert_eq!(matched, vec![PathBuf::from("/workspace/src/lib.rs")]);
    let _ = dir;
}

#[tokio::test]
async fn a_container_path_climbing_out_of_the_mount_is_refused() {
    let (_dir, fs) = workspace();

    let error = fs
        .read(Path::new("/workspace/../../etc/passwd"))
        .await
        .expect_err("must not resolve outside the mount");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(
        error.to_string().contains("leaves the workspace mount"),
        "{error}"
    );
}

#[tokio::test]
async fn a_scratch_path_says_it_has_no_host_file() {
    let (_dir, fs) = workspace();

    let error = fs
        .read(Path::new("/scratch/build.log"))
        .await
        .expect_err("scratch is a tmpfs inside the container");

    assert!(
        error.to_string().contains("no host path"),
        "the message has to explain why, or it reads as a missing file: {error}"
    );
}

/// `/workspaces/other` starts with the mount point as a *string* but is not
/// under it. Prefix matching without the separator would silently rewrite it.
#[tokio::test]
async fn a_path_that_merely_starts_with_the_mount_name_is_left_alone() {
    let (_dir, fs) = workspace();

    let translated = fs
        .to_host(Path::new("/workspaces/other"))
        .expect("passthrough");

    assert_eq!(translated, PathBuf::from("/workspaces/other"));
}

/// The mount point is duplicated between `exec.rs` (which builds the
/// `docker run` arguments) and `fs.rs` (which translates paths). If they ever
/// disagree the agent reads one tree and executes against another — the exact
/// failure #201 exists to remove — and nothing would report it.
#[test]
fn the_translated_mount_point_is_the_one_actually_mounted() {
    let request = archon_permissions::sandbox::SandboxCommandRequest {
        command: "true".into(),
        working_dir: PathBuf::from("/host/tree"),
        timeout_ms: 1_000,
        max_output_bytes: 1_024,
        env: Vec::new(),
    };
    let args = super::super::exec::docker_run_args(&DockerConfig::default(), "rw", &request);

    let mount = args
        .iter()
        .find(|arg| arg.starts_with("type=bind,"))
        .expect("the workspace bind mount");

    assert!(
        mount.ends_with(&format!("dst={CONTAINER_WORKSPACE}")),
        "exec.rs mounts somewhere fs.rs does not translate: {mount}"
    );
}
