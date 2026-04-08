/// TASK-HOOK-023: Dynamic File Watch Paths tests
use archon_core::hooks::{AggregatedHookResult, FileWatchManager, HookResult};
use tempfile::TempDir;

#[test]
fn test_watch_manager_creation() {
    let manager = FileWatchManager::new(100);
    assert_eq!(manager.watched_count(), 0);
}

#[test]
fn test_add_watch_paths() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    std::fs::write(dir.join("test.txt"), "hello").unwrap();

    let manager = FileWatchManager::new(100);
    manager.add_watch_paths(vec![dir.join("test.txt").to_string_lossy().to_string()]);
    assert_eq!(manager.watched_count(), 1);
}

#[test]
fn test_max_paths_enforced() {
    let tmp = TempDir::new().unwrap();
    let mut paths = Vec::new();
    for i in 0..110 {
        let p = tmp.path().join(format!("file{i}.txt"));
        std::fs::write(&p, "data").unwrap();
        paths.push(p.to_string_lossy().to_string());
    }

    let manager = FileWatchManager::new(100);
    manager.add_watch_paths(paths);
    assert_eq!(manager.watched_count(), 100, "max 100 paths enforced");
}

#[test]
fn test_clear_removes_all() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("watch.txt");
    std::fs::write(&p, "data").unwrap();

    let manager = FileWatchManager::new(100);
    manager.add_watch_paths(vec![p.to_string_lossy().to_string()]);
    assert_eq!(manager.watched_count(), 1);

    manager.clear();
    assert_eq!(manager.watched_count(), 0);
}

#[test]
fn test_duplicate_paths_not_double_counted() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("dup.txt");
    std::fs::write(&p, "data").unwrap();
    let path_str = p.to_string_lossy().to_string();

    let manager = FileWatchManager::new(100);
    manager.add_watch_paths(vec![path_str.clone(), path_str]);
    assert_eq!(
        manager.watched_count(),
        1,
        "duplicates should not be counted twice"
    );
}

#[test]
fn test_nonexistent_path_skipped() {
    let manager = FileWatchManager::new(100);
    manager.add_watch_paths(vec!["/nonexistent/path/file.txt".to_string()]);
    // Should not panic, may or may not add (depends on notify implementation)
    // Just verify no panic
}

#[test]
fn test_watch_paths_field_on_hook_result() {
    // Verify watch_paths field exists on HookResult
    let result = HookResult::default();
    assert!(result.watch_paths.is_empty());
}

#[test]
fn test_watch_paths_aggregated() {
    let mut agg = AggregatedHookResult::new();

    let mut r1 = HookResult::default();
    r1.watch_paths = vec!["/tmp/a.txt".into()];
    agg.merge(r1);

    let mut r2 = HookResult::default();
    r2.watch_paths = vec!["/tmp/b.txt".into()];
    agg.merge(r2);

    assert_eq!(agg.watch_paths.len(), 2);
}
