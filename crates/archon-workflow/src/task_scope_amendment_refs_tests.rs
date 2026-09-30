//! Batch O: a source file no task declares belongs to the tasks whose code
//! references it, to a fixpoint; only a file nothing references is unowned.

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

fn world(root: &Path) -> WorkflowV2TaskUniverse {
    write(root, "crates/w/Cargo.toml", "[package]\nname = \"w\"\n");
    write(
        root,
        "crates/w/src/lib.rs",
        "pub mod engine;\npub mod feeds;\npub mod store;\npub mod orphan;\npub mod a;\npub mod b;\npub mod caller;\npub mod replay;\n",
    );
    // A call to a function only one file defines.
    write(
        root,
        "crates/w/src/engine.rs",
        "pub struct Engine;\npub fn run() { replay_stored(); }\n#[cfg(test)]\n#[path = \"engine_tests.rs\"]\nmod tests;\n",
    );
    write(root, "crates/w/src/engine_tests.rs", "#[test]\nfn t() {}\n");
    // An integration test naming a declared item through a use list.
    write(
        root,
        "crates/w/tests/types.rs",
        "use w::engine::{Engine};\n#[test]\nfn t() { let _ = Engine; }\n",
    );
    write(
        root,
        "crates/w/src/replay.rs",
        "pub fn replay_stored() { super::feeds::archive::keep(); }\n",
    );
    // A child module, and a path through an unowned module to its child.
    write(
        root,
        "crates/w/src/store.rs",
        "mod cache;\nuse crate::feeds::daily::DailyFeed;\n",
    );
    write(root, "crates/w/src/store/cache.rs", "pub struct Cache;\n");
    write(
        root,
        "crates/w/src/feeds/mod.rs",
        "pub mod daily;\npub mod archive;\n",
    );
    write(
        root,
        "crates/w/src/feeds/daily.rs",
        "pub struct DailyFeed;\n#[path = \"daily_io.rs\"]\nmod io;\n",
    );
    write(root, "crates/w/src/feeds/daily_io.rs", "pub fn read() {}\n");
    write(root, "crates/w/src/feeds/archive.rs", "pub fn keep() {}\n");
    // Nothing references this file.
    write(root, "crates/w/src/orphan.rs", "pub struct Nobody;\n");
    // A name two files define references neither.
    write(root, "crates/w/src/a.rs", "pub fn shared() {}\n");
    write(root, "crates/w/src/b.rs", "pub fn shared() {}\n");
    write(root, "crates/w/src/caller.rs", "fn f() { shared(); }\n");
    // An integration test of declared code, with a module of its own.
    write(
        root,
        "crates/w/tests/engine_runs.rs",
        "mod support;\n#[test]\nfn t() { w::engine::run(); }\n",
    );
    write(
        root,
        "crates/w/tests/support/mod.rs",
        "pub fn fixture() {}\n",
    );
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("T-ENGINE", &["crates/w/src/engine.rs"]),
            task(
                "T-STORE",
                &["crates/w/src/store.rs", "crates/w/src/caller.rs"],
            ),
        ],
    }
}

#[test]
fn ownership_reaches_children_paths_calls_includes_and_tests_to_a_fixpoint() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let refs = referencing_tasks(&world(root), root);
    let owners = |file: &str| refs.get(file).cloned().unwrap_or_default();
    let one = |task: &str| BTreeSet::from([task.to_string()]);
    assert_eq!(owners("crates/w/src/replay.rs"), one("T-ENGINE"));
    assert_eq!(owners("crates/w/src/store/cache.rs"), one("T-STORE"));
    // `crate::feeds::daily::DailyFeed`: the path reaches the module's file.
    assert_eq!(owners("crates/w/src/feeds/daily.rs"), one("T-STORE"));
    // Transitively: a `#[path]` include of a newly owned file, and a
    // `super::` path from one.
    assert_eq!(owners("crates/w/src/feeds/daily_io.rs"), one("T-STORE"));
    // Used by the engine's code, but held in a module (`feeds`) no task's
    // code owns: shared infrastructure, left unowned.
    assert!(owners("crates/w/src/feeds/archive.rs").is_empty());
    // An integration test of declared code, and its own module.
    assert_eq!(
        owners("crates/w/tests/engine_runs.rs"),
        one("T-ENGINE"),
        "{refs:#?}"
    );
    assert_eq!(owners("crates/w/tests/support/mod.rs"), one("T-ENGINE"));
    // `#[path]` after another attribute, and a test naming a declared item.
    assert_eq!(owners("crates/w/src/engine_tests.rs"), one("T-ENGINE"));
    assert_eq!(owners("crates/w/tests/types.rs"), one("T-ENGINE"));
    // Nothing references these.
    assert!(owners("crates/w/src/orphan.rs").is_empty());
    assert!(owners("crates/w/src/a.rs").is_empty() && owners("crates/w/src/b.rs").is_empty());
    // The crate root is a hub: never spread from.
    assert!(owners("crates/w/src/lib.rs").is_empty());
}

#[test]
fn a_test_belongs_to_the_nearest_code_it_names_not_every_owner_of_what_it_touches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let universe = world(root);
    // A file both tasks' declared code uses directly: shared by the tie.
    write(
        root,
        "crates/w/src/lib.rs",
        "pub mod engine;\npub mod feeds;\npub mod store;\npub mod orphan;\npub mod a;\npub mod b;\npub mod caller;\npub mod replay;\npub mod common;\n",
    );
    write(root, "crates/w/src/common.rs", "pub struct Common;\n");
    write(
        root,
        "crates/w/src/engine.rs",
        "use crate::common::Common;\npub struct Engine;\npub fn run() { replay_stored(); }\n",
    );
    write(
        root,
        "crates/w/src/store.rs",
        "mod cache;\nuse crate::common::Common;\nuse crate::feeds::daily::DailyFeed;\n",
    );
    // A test of the engine that also touches the shared file.
    write(
        root,
        "crates/w/tests/engine_common.rs",
        "use w::common::Common;\n#[test]\nfn t() { w::engine::run(); let _ = Common; }\n",
    );
    // A test of only the shared file: its owners, the tie.
    write(
        root,
        "crates/w/tests/common_only.rs",
        "#[test]\nfn t() { let _ = w::common::Common; }\n",
    );
    let refs = referencing_tasks(&universe, root);
    let owners = |file: &str| refs.get(file).cloned().unwrap_or_default();
    let both = BTreeSet::from(["T-ENGINE".to_string(), "T-STORE".to_string()]);
    assert_eq!(owners("crates/w/src/common.rs"), both);
    assert_eq!(
        owners("crates/w/tests/engine_common.rs"),
        BTreeSet::from(["T-ENGINE".to_string()]),
        "{refs:#?}"
    );
    assert_eq!(owners("crates/w/tests/common_only.rs"), both);
}
