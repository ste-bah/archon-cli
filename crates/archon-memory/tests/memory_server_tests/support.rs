use super::*;

// ── helpers ────────────────────────────────────────────────────

/// Spin up an in-memory graph behind a TCP server, returning (port, graph, join-handle).
pub(super) async fn start_test_server(
    port_file: PathBuf,
) -> (u16, Arc<MemoryGraph>, tokio::task::JoinHandle<()>) {
    let graph = Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
    let (port, handle) = MemoryServer::start(Arc::clone(&graph), port_file)
        .await
        .expect("server start");
    (port, graph, handle)
}

pub(super) fn temp_port_file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("memory.port");
    (dir, path)
}
