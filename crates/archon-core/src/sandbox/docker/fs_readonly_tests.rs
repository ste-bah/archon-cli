//! A read-only workspace must be read-only for the file tools too.
//!
//! `DockerFs` serves the host bytes the bind mount exposes. For reads that is
//! the whole point. For writes it was a hole: `workspace_access = "ro"` — the
//! default — mounts `/workspace` read-only, so `Bash` cannot change a file,
//! while `Write`, `Edit`, `ApplyPatch` and `LargeEdit` translated the container
//! path back to a host path and changed it anyway. The setting read as enforced
//! and governed exactly one tool.
//!
//! Every test here is a pair: the refusal, and the read that must survive it.

use super::*;

fn read_only(dir: &Path) -> DockerFs {
    DockerFs::with_workspace_access(dir, "ro", &[])
}

fn seeded() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}").expect("seed");
    (dir, file)
}

#[tokio::test]
async fn a_read_only_workspace_refuses_a_write_but_still_serves_a_read() {
    let (dir, file) = seeded();
    let fs = read_only(dir.path());

    let refused = fs
        .write(Path::new("/workspace/main.rs"), b"tampered")
        .await
        .expect_err("a read-only mount must refuse the write the container refuses");
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(
        std::fs::read_to_string(&file).expect("host file"),
        "fn main() {}",
        "the host copy is the thing the mount was protecting"
    );

    assert_eq!(
        fs.read(Path::new("/workspace/main.rs"))
            .await
            .expect("reads are what a read-only mount is for"),
        b"fn main() {}",
        "gating writes must not cost the read"
    );
}

#[tokio::test]
async fn the_refusal_names_the_setting_that_caused_it() {
    let (dir, _file) = seeded();
    let message = read_only(dir.path())
        .write(Path::new("/workspace/main.rs"), b"tampered")
        .await
        .expect_err("refused")
        .to_string();

    assert!(
        message.contains("sandbox.workspace_access = \"ro\""),
        "a refusal a user cannot trace to a setting is just a broken tool: {message}"
    );
    assert!(
        message.contains("/workspace/main.rs"),
        "the refusal must name the path the caller asked for: {message}"
    );
}

#[tokio::test]
async fn a_read_write_workspace_writes_exactly_as_before() {
    let (dir, file) = seeded();
    let fs = DockerFs::with_workspace_access(dir.path(), "rw", &[]);

    fs.write(Path::new("/workspace/main.rs"), b"fn main() { todo!() }")
        .await
        .expect("rw is the mode that permits this");

    assert_eq!(
        std::fs::read_to_string(&file).expect("host file"),
        "fn main() { todo!() }"
    );
}

#[tokio::test]
async fn the_ungated_constructor_keeps_its_previous_behaviour() {
    let (dir, file) = seeded();

    DockerFs::new(dir.path())
        .write(Path::new("/workspace/main.rs"), b"unchanged behaviour")
        .await
        .expect("DockerFs::new carries no access mode and so gates nothing");

    assert_eq!(
        std::fs::read_to_string(&file).expect("host file"),
        "unchanged behaviour",
        "callers that cannot supply the mode must not silently start failing"
    );
}

#[tokio::test]
async fn scratch_mounts_the_workspace_read_only_and_so_does_this() {
    let (dir, _file) = seeded();
    // `workspace_mount_args` computes `readonly = workspace_access != "rw"`, so
    // "scratch" is a read-only workspace with a tmpfs beside it, not a writable
    // one. Reading that condition as "ro only" would leave the hole half open.
    let refused = DockerFs::with_workspace_access(dir.path(), "scratch", &[])
        .write(Path::new("/workspace/main.rs"), b"tampered")
        .await
        .expect_err("scratch mounts /workspace readonly too");

    assert!(
        refused.to_string().contains("\"scratch\""),
        "the message must name the mode actually configured: {refused}"
    );
}

#[tokio::test]
async fn every_mutating_operation_is_gated_not_just_write() {
    let (dir, _file) = seeded();
    std::fs::write(dir.path().join("other.rs"), "other").expect("seed");
    let fs = read_only(dir.path());

    for error in [
        fs.remove_file(Path::new("/workspace/main.rs"))
            .await
            .expect_err("remove"),
        fs.create_dir_all(Path::new("/workspace/src/new"))
            .await
            .expect_err("create_dir_all"),
        fs.rename(
            Path::new("/workspace/main.rs"),
            Path::new("/workspace/other.rs"),
        )
        .await
        .expect_err("rename"),
    ] {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::PermissionDenied,
            "a gate on `write` alone is a gate a caller walks around: {error}"
        );
    }

    assert!(
        dir.path().join("main.rs").exists(),
        "nothing may have been removed or moved"
    );
    assert!(!dir.path().join("src").exists());
}

#[tokio::test]
async fn a_rename_out_of_the_workspace_is_refused_at_the_source() {
    let (dir, _file) = seeded();
    let outside = tempfile::tempdir().expect("outside");
    let destination = outside.path().join("escaped.rs");

    let refused = read_only(dir.path())
        .rename(Path::new("/workspace/main.rs"), &destination)
        .await
        .expect_err("moving a file out of a read-only workspace still removes it from there");

    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(dir.path().join("main.rs").exists());
    assert!(!destination.exists());
}

#[tokio::test]
async fn writable_paths_stay_writable_under_a_read_only_workspace() {
    let (dir, _file) = seeded();
    std::fs::create_dir_all(dir.path().join("target")).expect("mkdir");
    let fs = DockerFs::with_workspace_access(dir.path(), "ro", &["target".to_string()]);

    fs.write(Path::new("/workspace/target/build.log"), b"ok")
        .await
        .expect("docker re-mounts writable_paths rw over the read-only workspace");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("target/build.log")).expect("written"),
        "ok"
    );

    fs.write(Path::new("/workspace/main.rs"), b"tampered")
        .await
        .expect_err("an exception for one directory is not an exception for the workspace");
}

#[tokio::test]
async fn a_writable_path_is_matched_by_component_not_by_string_prefix() {
    let (dir, _file) = seeded();
    std::fs::create_dir_all(dir.path().join("targeted")).expect("mkdir");
    let fs = DockerFs::with_workspace_access(dir.path(), "ro", &["target".to_string()]);

    fs.write(Path::new("/workspace/targeted/notes.txt"), b"tampered")
        .await
        .expect_err("`targeted` is not inside `target`, and a string prefix would say it is");
}

#[tokio::test]
async fn a_reroot_carries_the_read_only_mount_to_the_child() {
    let (dir, _file) = seeded();
    let worktree = dir.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("mkdir");
    std::fs::write(worktree.join("main.rs"), "child").expect("seed");

    let child = Arc::new(read_only(dir.path())).rerooted(&worktree);

    let refused = child
        .write(Path::new("/workspace/main.rs"), b"tampered")
        .await
        .expect_err("a subagent inheriting a permissive filesystem is the same hole, one step on");
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(
        child
            .read(Path::new("/workspace/main.rs"))
            .await
            .expect("the child still reads its own tree"),
        b"child"
    );
}

#[tokio::test]
async fn a_canonicalised_host_path_inside_the_workspace_is_still_gated() {
    let (dir, file) = seeded();
    // What a tool actually hands over: `path_guard` canonicalises before it
    // permits a write, and on macOS that rewrites `/var/...` to
    // `/private/var/...`. Checking containment against the configured spelling
    // alone would call this "outside the workspace" and wave it through.
    let canonical = file.canonicalize().expect("canonical target");

    let refused = read_only(dir.path())
        .write(&canonical, b"tampered")
        .await
        .expect_err("a host path is the form Write and Edit resolve to");

    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(
        std::fs::read_to_string(&file).expect("host file"),
        "fn main() {}"
    );
}

#[tokio::test]
async fn a_path_outside_the_workspace_is_left_to_the_host_path_guard() {
    let (dir, _file) = seeded();
    let outside = tempfile::tempdir().expect("outside");
    let target = outside.path().join("elsewhere.txt");

    // Not the mount's business: it bounds `/workspace`, and what may be touched
    // beyond it is `path_guard`'s question. Answering it here would be a second,
    // quieter confinement that nothing else knows about.
    read_only(dir.path())
        .write(&target, b"outside the mount")
        .await
        .expect("outside the workspace, this gate has no opinion");

    assert_eq!(
        std::fs::read_to_string(&target).expect("written"),
        "outside the mount"
    );
}
