//! Write confinement under a sandbox filesystem.
//!
//! The host guard returns early for a path the execution world names itself,
//! because a container path cannot be canonicalised on the host. That early
//! return skipped the write-root check, so an agent refused a host path could
//! write the identical file by spelling it the container's way. These tests
//! pin the fix from both directions: the bypass is closed, and the world paths
//! that are legitimately inside the roots still work.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{resolve_existing_write_target, resolve_write_target_path};
use crate::filesystem::{FileMeta, FileSystem, HostWriteTarget};
use crate::tool::ToolContext;

/// A bind-mounted world, modelled on `DockerFs`: `/workspace/x` and
/// `{root}/x` are the same bytes under two names.
#[derive(Debug)]
struct MountedWorld {
    root: PathBuf,
}

const MOUNT: &str = "/workspace";

impl MountedWorld {
    fn to_host(&self, path: &Path) -> Option<PathBuf> {
        let text = path.to_string_lossy().to_string();
        let relative = text.strip_prefix(&format!("{MOUNT}/"))?;
        // A `..` would climb out of the mount; the real world refuses it and so
        // does this, which is what makes `Unknown` the honest answer.
        if Path::new(relative)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return None;
        }
        Some(self.root.join(relative))
    }
}

/// A world whose files are on another machine entirely. It admits its paths and
/// cannot say what they mean here — the `RemoteFs` shape.
#[derive(Debug)]
struct RemoteWorld;

#[async_trait::async_trait]
impl FileSystem for MountedWorld {
    fn admit_world_path(&self, path: &Path) -> Option<std::io::Result<PathBuf>> {
        path.to_string_lossy()
            .starts_with(MOUNT)
            .then(|| Ok(path.to_path_buf()))
    }

    fn host_write_target(&self, path: &Path) -> HostWriteTarget {
        match self.to_host(path) {
            Some(host) => HostWriteTarget::Host(host),
            None => HostWriteTarget::Unknown,
        }
    }

    fn rerooted(self: Arc<Self>, _working_dir: &Path) -> Arc<dyn FileSystem> {
        self
    }

    async fn read(&self, _path: &Path) -> std::io::Result<Vec<u8>> {
        unimplemented!("the guard never reads")
    }
    async fn write(&self, _path: &Path, _contents: &[u8]) -> std::io::Result<()> {
        unimplemented!("the guard never writes")
    }
    async fn create_dir_all(&self, _path: &Path) -> std::io::Result<()> {
        unimplemented!()
    }
    async fn metadata(&self, _path: &Path) -> std::io::Result<FileMeta> {
        unimplemented!()
    }
    async fn read_dir(&self, _path: &Path) -> std::io::Result<Vec<PathBuf>> {
        unimplemented!()
    }
    async fn remove_file(&self, _path: &Path) -> std::io::Result<()> {
        unimplemented!()
    }
    async fn rename(&self, _from: &Path, _to: &Path) -> std::io::Result<()> {
        unimplemented!()
    }
    async fn glob(&self, _base: &Path, _pattern: &str) -> std::io::Result<Vec<PathBuf>> {
        unimplemented!()
    }
}

#[async_trait::async_trait]
impl FileSystem for RemoteWorld {
    fn admit_world_path(&self, path: &Path) -> Option<std::io::Result<PathBuf>> {
        path.to_string_lossy()
            .starts_with("/remote")
            .then(|| Ok(path.to_path_buf()))
    }

    fn rerooted(self: Arc<Self>, _working_dir: &Path) -> Arc<dyn FileSystem> {
        self
    }

    async fn read(&self, _path: &Path) -> std::io::Result<Vec<u8>> {
        unimplemented!()
    }
    async fn write(&self, _path: &Path, _contents: &[u8]) -> std::io::Result<()> {
        unimplemented!()
    }
    async fn create_dir_all(&self, _path: &Path) -> std::io::Result<()> {
        unimplemented!()
    }
    async fn metadata(&self, _path: &Path) -> std::io::Result<FileMeta> {
        unimplemented!()
    }
    async fn read_dir(&self, _path: &Path) -> std::io::Result<Vec<PathBuf>> {
        unimplemented!()
    }
    async fn remove_file(&self, _path: &Path) -> std::io::Result<()> {
        unimplemented!()
    }
    async fn rename(&self, _from: &Path, _to: &Path) -> std::io::Result<()> {
        unimplemented!()
    }
    async fn glob(&self, _base: &Path, _pattern: &str) -> std::io::Result<Vec<PathBuf>> {
        unimplemented!()
    }
}

struct Mounted {
    _root: tempfile::TempDir,
    /// The mounted tree. Writable only below `permitted`.
    mount: PathBuf,
    permitted: PathBuf,
}

/// A mount holding two directories, one of which the agent may write. Narrower
/// than the mount on purpose: if the roots were the whole mount, a container
/// path could not escape them even with the bypass open, and the test would
/// pass for the wrong reason.
fn mounted() -> Mounted {
    let root = tempfile::tempdir().expect("tempdir");
    let mount = root.path().join("mount");
    std::fs::create_dir_all(mount.join("mine")).expect("mine");
    std::fs::create_dir_all(mount.join("theirs")).expect("theirs");
    std::fs::write(mount.join("mine/ok.rs"), "// mine\n").expect("seed mine");
    std::fs::write(mount.join("theirs/no.rs"), "// theirs\n").expect("seed theirs");
    let mount = std::fs::canonicalize(&mount).expect("canonicalize");
    Mounted {
        _root: root,
        permitted: mount.join("mine"),
        mount,
    }
}

fn sandboxed(trees: &Mounted) -> ToolContext {
    ToolContext {
        working_dir: trees.mount.clone(),
        write_roots: vec![trees.permitted.clone()],
        fs: Some(Arc::new(MountedWorld {
            root: trees.mount.clone(),
        })),
        ..ToolContext::default()
    }
}

/// The bypass. Before this fix the container spelling of a refused host path
/// was permitted, because the guard returned before the write-root check.
#[test]
fn a_world_path_outside_the_write_roots_is_refused() {
    let trees = mounted();
    let ctx = sandboxed(&trees);

    let error = resolve_existing_write_target("/workspace/theirs/no.rs", &ctx)
        .expect_err("a container path must not evade write confinement");
    assert!(
        error.contains("outside this agent's writable directories"),
        "the refusal must say what rule was broken: {error}"
    );
}

/// The same file under its host name, refused identically. The two spellings
/// must give the same answer or the guard is decoration.
#[test]
fn a_host_path_outside_the_write_roots_is_refused_identically() {
    let trees = mounted();
    let ctx = sandboxed(&trees);
    let host = trees.mount.join("theirs/no.rs").display().to_string();

    let error = resolve_existing_write_target(&host, &ctx)
        .expect_err("the host spelling must be refused too");
    assert!(
        error.contains("outside this agent's writable directories"),
        "the refusal must say what rule was broken: {error}"
    );
}

/// Closing the bypass must not close the world. A container path inside the
/// roots is the ordinary case and still resolves to the world's own spelling,
/// which is what the model can reuse in `Bash`.
#[test]
fn a_world_path_inside_the_write_roots_is_permitted() {
    let trees = mounted();
    let ctx = sandboxed(&trees);

    let resolved = resolve_existing_write_target("/workspace/mine/ok.rs", &ctx)
        .expect("a container path inside the roots must still work");
    assert_eq!(
        resolved,
        PathBuf::from("/workspace/mine/ok.rs"),
        "the world path must come back in the world's spelling"
    );

    resolve_write_target_path("/workspace/mine/new.rs", &ctx)
        .expect("a new file inside the roots must be permitted");
}

/// Unconfined under a sandbox behaves exactly as it did before any of this
/// existed: the world vouches for its own paths and nothing else applies.
#[test]
fn an_unconfined_sandboxed_context_is_unchanged() {
    let trees = mounted();
    let mut ctx = sandboxed(&trees);
    ctx.write_roots.clear();

    resolve_existing_write_target("/workspace/theirs/no.rs", &ctx)
        .expect("with no write roots the world path is the world's business");
}

/// A world that cannot say where a write lands on this machine is refused
/// rather than waved through. Host directories are not a vocabulary a remote
/// workdir speaks, and pretending to compare them would be the inert guard
/// this work exists to remove.
#[test]
fn a_world_that_cannot_name_a_host_file_is_refused_under_confinement() {
    let trees = mounted();
    let ctx = ToolContext {
        working_dir: trees.mount.clone(),
        write_roots: vec![trees.permitted.clone()],
        fs: Some(Arc::new(RemoteWorld)),
        ..ToolContext::default()
    };

    let error = resolve_write_target_path("/remote/work/new.rs", &ctx)
        .expect_err("an unanswerable confinement question must not pass");
    assert!(
        error.contains("cannot say which file on this machine"),
        "the refusal must name why it could not be decided: {error}"
    );
}
