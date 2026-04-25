// TASK-AGS-303: Integration tests for LocalDiscoverySource.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use archon_core::agents::catalog::DiscoveryCatalog;
use archon_core::agents::discovery::local::LocalDiscoverySource;
use archon_core::agents::discovery::watcher::FsWatcher;
use archon_core::agents::metadata::AgentState;
use archon_core::agents::schema::AgentSchemaValidator;
use tempfile::TempDir;

/// Create a valid agent JSON file at the given path.
fn write_valid_agent(dir: &Path, name: &str) {
    let content = serde_json::json!({
        "name": name,
        "version": "1.0.0",
        "description": format!("Agent {name}"),
        "tags": ["test"],
        "capabilities": ["run"],
        "resource_requirements": {
            "cpu": 1.0,
            "memory_mb": 256,
            "timeout_sec": 60
        }
    });
    fs::write(dir.join(format!("{name}.json")), content.to_string()).unwrap();
}

/// Create an invalid agent JSON file (missing required `name` field).
fn write_invalid_agent(dir: &Path, filename: &str) {
    let content = serde_json::json!({
        "version": "1.0.0",
        "description": "missing name field",
        "resource_requirements": {
            "cpu": 1.0,
            "memory_mb": 256,
            "timeout_sec": 60
        }
    });
    fs::write(dir.join(filename), content.to_string()).unwrap();
}

#[test]
fn load_all_valid_agents() {
    let tmp = TempDir::new().unwrap();
    let categories = ["dev", "ops", "ml"];
    for (i, cat) in categories.iter().enumerate() {
        let dir = tmp.path().join(cat);
        fs::create_dir_all(&dir).unwrap();
        for j in 0..3 {
            write_valid_agent(&dir, &format!("agent-{i}-{j}"));
        }
    }
    // One more directly in root (uncategorized)
    write_valid_agent(tmp.path(), "root-agent");

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = DiscoveryCatalog::new();
    let source = LocalDiscoverySource::new(tmp.path().to_path_buf(), validator);

    let report = source.load_all(&catalog).unwrap();
    assert_eq!(report.loaded, 10);
    assert_eq!(report.invalid, 0);
    assert_eq!(catalog.len(), 10);
}

#[test]
fn load_all_with_invalid_files() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("agents");
    fs::create_dir_all(&dir).unwrap();

    // 3 valid
    for i in 0..3 {
        write_valid_agent(&dir, &format!("good-{i}"));
    }
    // 2 invalid (missing name)
    write_invalid_agent(&dir, "bad-1.json");
    write_invalid_agent(&dir, "bad-2.json");

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = DiscoveryCatalog::new();
    let source = LocalDiscoverySource::new(tmp.path().to_path_buf(), validator);

    let report = source.load_all(&catalog).unwrap();
    assert_eq!(report.loaded, 3);
    assert_eq!(report.invalid, 2);
    // All 5 are in the catalog (invalid ones preserved per EC-DISCOVERY-001)
    assert_eq!(catalog.len(), 5);

    // Verify invalid entries have state=Invalid
    let snap = catalog.snapshot();
    let mut invalid_count = 0;
    for entry in snap.entries.iter() {
        if matches!(&entry.value().state, AgentState::Invalid(_)) {
            invalid_count += 1;
        }
    }
    assert_eq!(invalid_count, 2);
}

#[test]
fn watcher_detects_new_file() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("agents");
    fs::create_dir_all(&dir).unwrap();
    write_valid_agent(&dir, "initial");

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = Arc::new(DiscoveryCatalog::new());
    let source = Arc::new(LocalDiscoverySource::new(
        tmp.path().to_path_buf(),
        validator,
    ));

    // Initial load
    source.load_all(&catalog).unwrap();
    assert_eq!(catalog.len(), 1);

    // Start watcher
    let _watcher = FsWatcher::start(tmp.path(), source, catalog.clone()).unwrap();

    // Write a new agent file
    write_valid_agent(&dir, "new-agent");

    // Wait for debounce + rescan (250ms should be enough for 100ms debounce)
    std::thread::sleep(std::time::Duration::from_millis(350));

    // Catalog should now have 2 entries
    assert!(
        catalog.len() >= 2,
        "expected >= 2 after watcher, got {}",
        catalog.len()
    );
}

#[test]
fn perf_300_agents_under_1s() {
    let tmp = TempDir::new().unwrap();
    let categories = ["dev", "ops", "ml", "infra", "test", "core"];
    for (i, cat) in categories.iter().enumerate() {
        let dir = tmp.path().join(cat);
        fs::create_dir_all(&dir).unwrap();
        for j in 0..50 {
            write_valid_agent(&dir, &format!("agent-{i}-{j}"));
        }
    }

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = DiscoveryCatalog::new();
    let source = LocalDiscoverySource::new(tmp.path().to_path_buf(), validator);

    let report = source.load_all(&catalog).unwrap();
    assert_eq!(report.loaded, 300);
    assert!(
        report.duration_ms < 1000,
        "load_all took {}ms, expected <1000ms",
        report.duration_ms
    );
}
