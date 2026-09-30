//! Batch O: a source file no task declares belongs to the tasks whose
//! declared code references it; a file nothing references is dead code.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;

fn write(root: &Path, path: &str, text: &str) {
    let target = root.join(path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, text).unwrap();
}

fn task(id: &str, owns: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn declared_code_references_reach_child_modules_used_items_and_called_functions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "crates/w/Cargo.toml", "[package]\nname = \"w\"\n");
    write(
        root,
        "crates/w/src/lib.rs",
        "pub mod engine;\npub mod feeds;\npub mod store;\n",
    );
    // The engine calls a function only the replay file defines.
    write(
        root,
        "crates/w/src/engine.rs",
        "pub fn run() { replay_stored(); }\n",
    );
    write(
        root,
        "crates/w/src/replay.rs",
        "pub fn replay_stored() {}\n",
    );
    // The store names a type through a path into an unowned module's child.
    write(
        root,
        "crates/w/src/store.rs",
        "mod cache;\nuse crate::feeds::{DailyFeed};\n",
    );
    write(root, "crates/w/src/store/cache.rs", "pub struct Cache;\n");
    write(root, "crates/w/src/feeds/mod.rs", "pub mod daily;\n");
    write(
        root,
        "crates/w/src/feeds/daily.rs",
        "pub struct DailyFeed;\n",
    );
    // A file nothing references.
    write(root, "crates/w/src/orphan.rs", "pub struct Nobody;\n");
    // A name two files define references neither.
    write(root, "crates/w/src/a.rs", "pub fn shared() {}\n");
    write(root, "crates/w/src/b.rs", "pub fn shared() {}\n");
    write(root, "crates/w/src/caller.rs", "fn f() { shared(); }\n");
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("T-ENGINE", &["crates/w/src/engine.rs"]),
            task(
                "T-STORE",
                &["crates/w/src/store.rs", "crates/w/src/caller.rs"],
            ),
        ],
    };
    let refs = referencing_tasks(&universe, root);
    let owners = |file: &str| refs.get(file).cloned().unwrap_or_default();
    assert_eq!(
        owners("crates/w/src/replay.rs"),
        BTreeSet::from(["T-ENGINE".to_string()])
    );
    assert_eq!(
        owners("crates/w/src/store/cache.rs"),
        BTreeSet::from(["T-STORE".to_string()])
    );
    assert_eq!(
        owners("crates/w/src/feeds/daily.rs"),
        BTreeSet::from(["T-STORE".to_string()])
    );
    assert!(owners("crates/w/src/orphan.rs").is_empty());
    assert!(owners("crates/w/src/a.rs").is_empty() && owners("crates/w/src/b.rs").is_empty());
}
