//! LargeEdit must land in the execution world, not on the host behind it.
//!
//! `Write`, `Edit` and `ApplyPatch` all go through `ToolContext::fs`. LargeEdit
//! used `std::fs` directly, so under any sandbox whose filesystem is not the
//! host it staged and committed to the host disk while the agent's shell looked
//! at a different tree — silently, because every operation succeeded.
//!
//! A recorder alone cannot catch that: `LocalFs` and a direct `std::fs` call
//! touch the same bytes, so a test built on the host filesystem passes either
//! way. [`RelocatingFs`] is therefore a world that is *demonstrably somewhere
//! else* — it serves a second directory — and the assertions are about which of
//! the two directories changed.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::filesystem::{FileMeta, FileSystem, LocalFs};
use crate::large_edit::{LargeEditBeginTool, LargeEditCommitTool, LargeEditReplaceSectionTool};
use crate::tool::{Tool, ToolContext};

/// A world that holds the same names in a different directory.
///
/// Every path under `host_root` is served from `world_root`, which is exactly
/// the shape of a sandbox whose tree is not the host's. Results are mapped back
/// so callers keep seeing the names they used.
#[derive(Debug)]
struct RelocatingFs {
    host_root: PathBuf,
    world_root: PathBuf,
    ops: Arc<Mutex<Vec<String>>>,
}

impl RelocatingFs {
    fn new(host_root: &Path, world_root: &Path) -> Self {
        Self {
            host_root: host_root.to_path_buf(),
            world_root: world_root.to_path_buf(),
            ops: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn into_world(&self, path: &Path) -> PathBuf {
        match path.strip_prefix(&self.host_root) {
            Ok(relative) => self.world_root.join(relative),
            Err(_) => path.to_path_buf(),
        }
    }

    fn out_of_world(&self, path: &Path) -> PathBuf {
        match path.strip_prefix(&self.world_root) {
            Ok(relative) => self.host_root.join(relative),
            Err(_) => path.to_path_buf(),
        }
    }

    fn record(&self, operation: &str, path: &Path) {
        self.ops
            .lock()
            .expect("ops lock")
            .push(format!("{operation} {}", path.display()));
    }
}

#[async_trait::async_trait]
impl FileSystem for RelocatingFs {
    async fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.record("read", path);
        LocalFs.read(&self.into_world(path)).await
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        self.record("write", path);
        LocalFs.write(&self.into_world(path), contents).await
    }

    async fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.record("create_dir_all", path);
        LocalFs.create_dir_all(&self.into_world(path)).await
    }

    async fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        LocalFs.metadata(&self.into_world(path)).await
    }

    async fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let entries = LocalFs.read_dir(&self.into_world(path)).await?;
        Ok(entries
            .into_iter()
            .map(|entry| self.out_of_world(&entry))
            .collect())
    }

    async fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.record("remove_file", path);
        LocalFs.remove_file(&self.into_world(path)).await
    }

    async fn remove_dir(&self, path: &Path) -> io::Result<()> {
        self.record("remove_dir", path);
        LocalFs.remove_dir(&self.into_world(path)).await
    }

    async fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.record("rename", to);
        LocalFs
            .rename(&self.into_world(from), &self.into_world(to))
            .await
    }

    fn rerooted(self: Arc<Self>, _working_dir: &Path) -> Arc<dyn FileSystem> {
        self
    }

    async fn glob(&self, base: &Path, pattern: &str) -> io::Result<Vec<PathBuf>> {
        let matched = LocalFs.glob(&self.into_world(base), pattern).await?;
        Ok(matched
            .into_iter()
            .map(|entry| self.out_of_world(&entry))
            .collect())
    }
}

struct Worlds {
    _host: tempfile::TempDir,
    _world: tempfile::TempDir,
    host_root: PathBuf,
    world_root: PathBuf,
    ops: Arc<Mutex<Vec<String>>>,
    ctx: ToolContext,
}

/// Two directories holding `doc.md` with identical bytes, and a context whose
/// filesystem serves the second one.
///
/// Identical bytes matter: `commit` re-hashes the target before it writes, so a
/// world copy that differed would fail the freshness check for the wrong reason
/// and hide whatever the test was actually asking.
fn worlds(contents: &str) -> Worlds {
    let host = tempfile::tempdir().expect("host tempdir");
    let world = tempfile::tempdir().expect("world tempdir");
    // Canonicalised, because the path guard canonicalises the target it
    // resolves. On macOS a temp dir is `/var/...` and its real path is
    // `/private/var/...`; a world keyed on the uncanonicalised root would fail
    // to recognise its own file and silently fall through to the host — which
    // is precisely the outcome these tests exist to distinguish.
    let host_root = host.path().canonicalize().expect("host root");
    let world_root = world.path().canonicalize().expect("world root");
    std::fs::write(host_root.join("doc.md"), contents).expect("host seed");
    std::fs::write(world_root.join("doc.md"), contents).expect("world seed");

    let fs = Arc::new(RelocatingFs::new(&host_root, &world_root));
    let ops = Arc::clone(&fs.ops);
    let ctx = ToolContext {
        working_dir: host_root.clone(),
        session_id: "large-edit-world-test".into(),
        fs: Some(fs),
        ..Default::default()
    };
    Worlds {
        _host: host,
        _world: world,
        host_root,
        world_root,
        ops,
        ctx,
    }
}

async fn begin_edit(worlds: &Worlds) -> String {
    let begun = LargeEditBeginTool
        .execute(
            json!({ "file_path": worlds.host_root.join("doc.md").display().to_string() }),
            &worlds.ctx,
        )
        .await;
    assert!(!begun.is_error, "{}", begun.content);
    serde_json::from_str::<serde_json::Value>(&begun.content).expect("begin json")["edit_id"]
        .as_str()
        .expect("edit_id")
        .to_string()
}

#[tokio::test]
async fn large_edit_commit_writes_the_world_not_the_host() {
    let worlds = worlds("# A\nold\n# B\nkeep\n");
    let edit_id = begin_edit(&worlds).await;

    let replaced = LargeEditReplaceSectionTool
        .execute(
            json!({ "edit_id": edit_id, "start_anchor": "# A", "content": "# A\nnew\n" }),
            &worlds.ctx,
        )
        .await;
    assert!(!replaced.is_error, "{}", replaced.content);

    let committed = LargeEditCommitTool
        .execute(
            json!({ "edit_id": edit_id, "required_fragments": ["new"] }),
            &worlds.ctx,
        )
        .await;
    assert!(!committed.is_error, "{}", committed.content);

    assert_eq!(
        std::fs::read_to_string(worlds.world_root.join("doc.md")).expect("world target"),
        "# A\nnew\n# B\nkeep\n",
        "the commit must land in the world ToolContext::fs serves"
    );
    assert_eq!(
        std::fs::read_to_string(worlds.host_root.join("doc.md")).expect("host target"),
        "# A\nold\n# B\nkeep\n",
        "the host copy is what a sandboxed agent must not have been able to touch"
    );
}

#[tokio::test]
async fn large_edit_staging_never_touches_the_host_tree() {
    let worlds = worlds("# A\nold\n");
    let edit_id = begin_edit(&worlds).await;

    assert!(
        worlds
            .world_root
            .join(".archon/large-edits")
            .join(&edit_id)
            .join("staged.txt")
            .exists(),
        "the staged copy belongs in the world"
    );
    assert!(
        !worlds.host_root.join(".archon").exists(),
        "staging wrote the host tree, which is the bypass this test exists for"
    );
}

#[tokio::test]
async fn large_edit_commit_replaces_the_target_by_world_rename() {
    let worlds = worlds("# A\nold\n");
    let edit_id = begin_edit(&worlds).await;

    let replaced = LargeEditReplaceSectionTool
        .execute(
            json!({ "edit_id": edit_id, "start_anchor": "# A", "content": "# A\nnew\n" }),
            &worlds.ctx,
        )
        .await;
    assert!(!replaced.is_error, "{}", replaced.content);
    let committed = LargeEditCommitTool
        .execute(json!({ "edit_id": edit_id }), &worlds.ctx)
        .await;
    assert!(!committed.is_error, "{}", committed.content);

    let ops = worlds.ops.lock().expect("ops lock").clone();
    let target = worlds.host_root.join("doc.md").display().to_string();
    assert!(
        ops.contains(&format!("rename {target}")),
        "the write-temp-then-rename dance must survive the move onto FileSystem, \
         so a crash mid-commit cannot leave a half-written target: {ops:?}"
    );
    assert!(
        !ops.contains(&format!("write {target}")),
        "the target must never be written in place: {ops:?}"
    );
}

/// An aborted edit must leave nothing behind — in the WORLD, not the host.
///
/// The session directory used to survive every abort because the filesystem
/// trait had no way to remove a directory. One husk is inert; one per large
/// edit accumulates in the working tree indefinitely. Asserted through the
/// relocating world so a fix that reached past `ctx.fs()` to `std::fs` — the
/// original bug in this module — would fail rather than pass.
#[tokio::test]
async fn an_aborted_edit_leaves_no_session_directory_in_the_world() {
    let worlds = worlds("# A\nold\n# B\nkeep\n");
    let edit_id = begin_edit(&worlds).await;

    let sessions = worlds.world_root.join(".archon").join("large-edits");
    let session_dir = sessions.join(&edit_id);
    assert!(
        session_dir.is_dir(),
        "the session directory should exist in the world before the abort: {}",
        session_dir.display()
    );

    let aborted = crate::large_edit::LargeEditAbortTool
        .execute(serde_json::json!({ "edit_id": edit_id }), &worlds.ctx)
        .await;
    assert!(!aborted.is_error, "{}", aborted.content);

    assert!(
        !session_dir.exists(),
        "an aborted edit must not leave its session directory behind: {}",
        session_dir.display()
    );
    assert!(
        worlds
            .ops
            .lock()
            .expect("ops")
            .iter()
            .any(|op| op.starts_with("remove_dir")),
        "the removal must go through the world's filesystem, not the host's"
    );
}
