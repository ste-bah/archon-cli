//! The tools themselves, not the filesystem underneath them (#201).
//!
//! `sandbox_docker_world.rs` drives `DockerFs` directly, which proves the
//! translation and nothing about the path a tool would actually accept. Between
//! the two sits `path_guard`, which canonicalises on the host and checks host
//! roots — so `Read` refused `/workspace/src/main.rs` outright while the
//! filesystem beneath it would have resolved the file perfectly. Every test
//! here goes through `Tool::execute`, which is the only way that gap is visible.
//!
//! No Docker daemon is needed: `DockerFs` is a bind-mount translation, so the
//! bytes are the host's either way. What is under test is which paths the tools
//! accept and where they land, and that is decided entirely in-process.

use std::sync::Arc;

use archon_core::sandbox::DockerFs;
use archon_tools::file_read::ReadTool;
use archon_tools::file_write::WriteTool;
use archon_tools::filesystem::FileSystem;
use archon_tools::tool::{Tool, ToolContext};

fn sandboxed_ctx(working_dir: &std::path::Path) -> ToolContext {
    ToolContext {
        working_dir: working_dir.to_path_buf(),
        fs: Some(Arc::new(DockerFs::new(working_dir)) as Arc<dyn FileSystem>),
        ..Default::default()
    }
}

#[tokio::test]
async fn read_accepts_the_container_path_bash_printed() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").expect("write");
    let ctx = sandboxed_ctx(dir.path());

    let result = ReadTool
        .execute(
            serde_json::json!({ "file_path": "/workspace/src/main.rs" }),
            &ctx,
        )
        .await;

    assert!(
        !result.is_error,
        "Read refused the path the container named: {}",
        result.content
    );
    assert!(
        result.content.contains("fn main() {}"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn write_accepts_the_container_path_and_lands_in_the_workspace() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = sandboxed_ctx(dir.path());

    let result = WriteTool
        .execute(
            serde_json::json!({
                "file_path": "/workspace/written.txt",
                "content": "through the tool",
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("written.txt")).expect("host read"),
        "through the tool",
        "the write did not land in the mounted workspace"
    );
}

/// The host guard still applies to host paths. Admitting the world's paths must
/// not become a way to name any file on the machine.
#[tokio::test]
async fn a_host_path_outside_the_workspace_is_still_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("secret.txt"), "not yours").expect("write");
    let ctx = sandboxed_ctx(dir.path());

    let result = ReadTool
        .execute(
            serde_json::json!({
                "file_path": outside.path().join("secret.txt").display().to_string(),
            }),
            &ctx,
        )
        .await;

    assert!(
        result.is_error,
        "a sandboxed session read a file outside its workspace: {}",
        result.content
    );
    assert!(
        result.content.contains("outside allowed directories"),
        "{}",
        result.content
    );
}

/// A container path that climbs out of the mount is refused by the world, and
/// the tool reports that rather than resolving it.
#[tokio::test]
async fn a_container_path_leaving_the_mount_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = sandboxed_ctx(dir.path());

    let result = ReadTool
        .execute(
            serde_json::json!({ "file_path": "/workspace/../../etc/passwd" }),
            &ctx,
        )
        .await;

    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.contains("leaves the workspace mount"),
        "{}",
        result.content
    );
}

/// Host paths inside the workspace keep working exactly as before — the common
/// case, and the one a change here could quietly break.
#[tokio::test]
async fn a_host_path_inside_the_workspace_still_works() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("plain.txt");
    std::fs::write(&file, "host bytes").expect("write");
    let ctx = sandboxed_ctx(dir.path());

    let result = ReadTool
        .execute(
            serde_json::json!({ "file_path": file.display().to_string() }),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("host bytes"), "{}", result.content);
}

/// With no sandbox configured a container path is meaningless and must stay
/// refused: nothing should start accepting `/workspace` on an ordinary session.
#[tokio::test]
async fn without_a_sandbox_a_container_path_is_still_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = ToolContext {
        working_dir: dir.path().to_path_buf(),
        ..Default::default()
    };

    let result = ReadTool
        .execute(
            serde_json::json!({ "file_path": "/workspace/src/main.rs" }),
            &ctx,
        )
        .await;

    assert!(
        result.is_error,
        "an unsandboxed session resolved a container path: {}",
        result.content
    );
}
