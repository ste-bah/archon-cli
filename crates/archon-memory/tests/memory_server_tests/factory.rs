use super::*;
// Access factory tests
// ═══════════════════════════════════════════════════════════════

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_memory_first_session_becomes_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let access = open_memory(dir.path()).await.expect("open");
    assert!(
        matches!(access, MemoryAccess::Direct { .. }),
        "first session should be Direct"
    );

    // Port file should exist
    let port_file = dir.path().join("memory.port");
    assert!(port_file.exists(), "port file should be created");

    // Should function as a memory store
    let mt: &dyn MemoryTrait = &access;
    let id = mt
        .store_memory(
            "factory test",
            "ft",
            MemoryType::Fact,
            0.5,
            &[],
            "t",
            "/tmp",
        )
        .expect("store");
    let mem = mt.get_memory(&id).expect("get");
    assert_eq!(mem.content, "factory test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_memory_second_session_becomes_client() {
    let dir = tempfile::tempdir().expect("tempdir");

    // First session starts the server
    let _first = open_memory(dir.path()).await.expect("first open");
    assert!(matches!(_first, MemoryAccess::Direct { .. }));

    // Second session should connect as client
    let second = open_memory(dir.path()).await.expect("second open");
    assert!(
        matches!(second, MemoryAccess::Remote(_)),
        "second session should be Remote"
    );

    // Store via first, retrieve via second
    let mt1: &dyn MemoryTrait = &_first;
    let id = mt1
        .store_memory(
            "shared memory",
            "sm",
            MemoryType::Fact,
            0.8,
            &[],
            "t",
            "/tmp",
        )
        .expect("store via first");

    let mt2: &dyn MemoryTrait = &second;
    let mem = mt2.get_memory(&id).expect("get via second");
    assert_eq!(mem.content, "shared memory");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_memory_stale_port_cleaned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port_file = dir.path().join("memory.port");

    // Write a stale port file pointing to a port nothing listens on
    std::fs::write(&port_file, "65432").expect("write stale port");
    assert!(port_file.exists());

    // open_memory should detect the stale port, clean it up, and become server
    let access = open_memory(dir.path()).await.expect("open with stale port");
    assert!(
        matches!(access, MemoryAccess::Direct { .. }),
        "should become server after cleaning stale port"
    );

    // New port file should be written with the actual port
    let contents = std::fs::read_to_string(&port_file).expect("read port file");
    let port: u16 = contents.trim().parse().expect("valid port");
    assert_ne!(port, 65432, "should not still have stale port");
}
