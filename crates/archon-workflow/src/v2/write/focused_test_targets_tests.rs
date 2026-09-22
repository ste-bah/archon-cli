use std::path::Path;

use super::{
    FocusedTestTarget, FocusedTestTargets, Resolution, Selection, Widenable, preamble, resolution,
    selection, stamp_result, widenable,
};
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn write_text(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn write(root: &Path, rel: &str) {
    write_text(root, rel, "// module\n");
}

/// A workspace with a root package `app` and a member crate `engine`:
/// `engine` holds `store.rs` with a `store/` directory, a `#[path]`
/// sibling `store/cases_tests.rs`, a `plan/mod.rs` module, and an
/// integration test `tests/backtest.rs`.
fn workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_text(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\n\n[package]\nname = \"app\"\n",
    );
    write(root, "src/main.rs");
    write(root, "src/command.rs");
    write_text(
        root,
        "crates/engine/Cargo.toml",
        "[package]\nname = \"engine\"\nversion = \"0.1.0\"\n",
    );
    write(root, "crates/engine/src/lib.rs");
    write(root, "crates/engine/src/store.rs");
    write(root, "crates/engine/src/store/cases.rs");
    write(root, "crates/engine/src/store/cases_tests.rs");
    write(root, "crates/engine/src/plan/mod.rs");
    write(root, "crates/engine/tests/backtest.rs");
    temp
}

/// The one target `command` resolves to, or `None` for nothing or a tie.
fn resolve(root: &Path, command: &str) -> Option<FocusedTestTarget> {
    match resolution(root, command) {
        Resolution::One(target) => Some(target),
        _ => None,
    }
}

fn target(file: &str, dir: &str) -> Option<FocusedTestTarget> {
    Some(FocusedTestTarget {
        file: file.into(),
        dir: dir.into(),
    })
}

#[test]
fn the_selection_is_the_positional_filter_after_the_options() {
    assert_eq!(
        selection("cargo nextest run -p engine --lib store::cases::strategy"),
        Selection::Module(vec!["store".into(), "cases".into(), "strategy".into()])
    );
    assert_eq!(
        selection("cargo test -p engine --features x --lib store::cases -- --nocapture"),
        Selection::Module(vec!["store".into(), "cases".into()])
    );
    assert_eq!(
        selection("timeout 900 cargo test --package=engine -j 4 store::cases"),
        Selection::Module(vec!["store".into(), "cases".into()])
    );
    assert_eq!(
        selection("cargo nextest run -p engine --test backtest"),
        Selection::IntegrationTest("backtest".into())
    );
    assert_eq!(
        selection("cargo test -p engine --test=backtest smoke::one"),
        Selection::IntegrationTest("backtest".into())
    );
    // A bare name is a one-segment filter (Issue-72); a filterset value, no
    // filter, and non-test commands select nothing.
    assert_eq!(
        selection("cargo test -p engine smoke"),
        Selection::Module(vec!["smoke".into()])
    );
    assert_eq!(
        selection("cargo nextest run -p engine -E 'test(store::cases)'"),
        Selection::None
    );
    assert_eq!(selection("cargo test -p engine --lib"), Selection::None);
    assert_eq!(
        selection("cargo clippy -p engine -- -D warnings"),
        Selection::None
    );
    assert_eq!(selection("cargo fmt -p engine -- --check"), Selection::None);
    assert_eq!(selection("npm test -- store::cases"), Selection::None);
    assert_eq!(
        selection("cargo test -p engine store::bad-name"),
        Selection::None
    );
    assert_eq!(
        selection("cargo test -p engine --test ../escape"),
        Selection::None
    );
}

#[test]
fn a_module_filter_resolves_to_its_file_and_module_directory() {
    let ws = workspace();
    assert_eq!(
        resolve(ws.path(), "cargo nextest run -p engine --lib store::cases"),
        target(
            "crates/engine/src/store/cases.rs",
            "crates/engine/src/store/cases"
        )
    );
    assert_eq!(
        resolve(ws.path(), "cargo test -p engine --lib store"),
        target("crates/engine/src/store.rs", "crates/engine/src/store"),
        "a bare filter is a one-segment module path (Issue-72)"
    );
    assert_eq!(
        resolve(ws.path(), "cargo test -p engine --lib plan::tests"),
        target("crates/engine/src/plan/mod.rs", "crates/engine/src/plan")
    );
    // A `tests` segment reaches the `#[path]` sibling file.
    assert_eq!(
        resolve(
            ws.path(),
            "cargo test -p engine store::cases::tests::round_trips"
        ),
        target(
            "crates/engine/src/store/cases_tests.rs",
            "crates/engine/src/store/cases_tests"
        )
    );
}

#[test]
fn a_test_function_filter_is_treated_as_its_module() {
    let ws = workspace();
    assert_eq!(
        resolve(
            ws.path(),
            "cargo test -p engine --lib store::cases::round_trips"
        ),
        target(
            "crates/engine/src/store/cases.rs",
            "crates/engine/src/store/cases"
        )
    );
}

#[test]
fn an_integration_test_resolves_to_its_tests_file_and_directory() {
    let ws = workspace();
    assert_eq!(
        resolve(ws.path(), "cargo nextest run -p engine --test backtest"),
        target(
            "crates/engine/tests/backtest.rs",
            "crates/engine/tests/backtest"
        )
    );
    assert_eq!(
        resolve(ws.path(), "cargo nextest run -p engine --test missing"),
        None
    );
    // No package named: exactly one package holds the file.
    assert_eq!(
        resolve(ws.path(), "cargo test --test backtest"),
        target(
            "crates/engine/tests/backtest.rs",
            "crates/engine/tests/backtest"
        )
    );
}

#[test]
fn clippy_fmt_and_foreign_commands_resolve_nothing() {
    let ws = workspace();
    for command in [
        "cargo clippy -p engine --all-targets -- -D warnings",
        "cargo fmt -p engine -- --check",
        "cargo build -p engine",
        "npm test",
    ] {
        assert_eq!(resolve(ws.path(), command), None, "{command}");
    }
}

#[test]
fn a_missing_leaf_module_lands_on_the_parent_module_file_and_its_directory() {
    let ws = workspace();
    // `store::strategy_spec` does not exist yet: the coder will create it
    // under `store/` and declare it in `store.rs`, so both are widened.
    assert_eq!(
        resolve(ws.path(), "cargo test -p engine --lib store::strategy_spec"),
        target("crates/engine/src/store.rs", "crates/engine/src/store")
    );
    // No module on the path exists: nothing.
    assert_eq!(
        resolve(ws.path(), "cargo test -p engine --lib nowhere::at_all"),
        None
    );
    // The crate root is never widened.
    assert_eq!(
        resolve(ws.path(), "cargo test -p engine --lib tests::smoke"),
        None
    );
    // A package the workspace does not hold: nothing.
    assert_eq!(
        resolve(ws.path(), "cargo test -p ghost --lib store::cases"),
        None
    );
}

fn universe(tasks: &[(&str, &[&str])]) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: tasks
            .iter()
            .map(|(id, files)| WorkflowV2TaskUniverseTask {
                canonical_task_id: (*id).into(),
                source_path: format!("tasks/{id}.md"),
                files_expected_to_change: files.iter().map(|f| f.to_string()).collect(),
                ..Default::default()
            })
            .collect(),
    }
}

#[test]
fn a_file_another_task_declares_is_not_widened_and_neither_is_its_directory() {
    let ws = workspace();
    let universe = universe(&[
        ("TASK-A", &["crates/engine/src/lib.rs"]),
        ("TASK-B", &["crates/engine/src/store/cases.rs"]),
        ("TASK-C", &["crates/engine/tests/backtest/fixtures.rs"]),
    ]);
    let own = vec!["TASK-A".to_string()];
    let commands = vec![
        "cargo test -p engine --lib store::cases".to_string(),
        "cargo test -p engine --lib plan::tests".to_string(),
        "cargo test -p engine --test backtest".to_string(),
        "cargo clippy -p engine".to_string(),
    ];
    let widened = widenable(ws.path(), Some(&universe), &own, &[], &commands);
    assert_eq!(
        widened,
        Widenable {
            files: vec![
                "crates/engine/src/plan/mod.rs".into(),
                "crates/engine/tests/backtest.rs".into(),
            ],
            // `backtest/` holds a file TASK-C declares: the file is widened,
            // the directory is not.
            dirs: vec!["crates/engine/src/plan".into()],
            ambiguous: Vec::new(),
        }
    );
    // The declaring task itself is widened to its own file and directory.
    let own_b = vec!["TASK-B".to_string()];
    let widened = widenable(ws.path(), Some(&universe), &own_b, &[], &commands[..1]);
    assert_eq!(
        widened,
        Widenable {
            files: vec!["crates/engine/src/store/cases.rs".into()],
            dirs: vec!["crates/engine/src/store/cases".into()],
            ambiguous: Vec::new(),
        }
    );
    // Without a universe, everything resolved is the branch's.
    let widened = widenable(ws.path(), None, &own, &[], &commands[..1]);
    assert_eq!(
        widened.files,
        vec!["crates/engine/src/store/cases.rs".to_string()]
    );
}

/// A package `trading` holding the given module files under `src/`.
fn package_with(files: &[&str]) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_text(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\n",
    );
    write_text(
        root,
        "crates/trading/Cargo.toml",
        "[package]\nname = \"trading\"\nversion = \"0.1.0\"\n",
    );
    write(root, "crates/trading/src/lib.rs");
    for file in files {
        write(root, &format!("crates/trading/src/{file}"));
    }
    temp
}

#[test]
fn a_suffix_filter_resolves_to_the_module_whose_path_ends_with_it() {
    // Issue-72, the live shape: the filter names the last two segments of
    // `data_store::data_store_ahdm_tests::pine`.
    let ws = package_with(&[
        "data_store.rs",
        "data_store/data_store_ahdm_tests.rs",
        "data_store/data_store_ahdm_tests/pine.rs",
        "data_store/data_store_ahdm_tests/gold.rs",
    ]);
    let pine = target(
        "crates/trading/src/data_store/data_store_ahdm_tests/pine.rs",
        "crates/trading/src/data_store/data_store_ahdm_tests/pine",
    );
    assert_eq!(
        resolve(
            ws.path(),
            "cargo nextest run -p archon-trading --lib data_store_ahdm_tests::pine"
        ),
        None,
        "a package the workspace does not hold still resolves nothing"
    );
    assert_eq!(
        resolve(
            ws.path(),
            "cargo nextest run -p trading --lib data_store_ahdm_tests::pine"
        ),
        pine
    );
    // A test-function filter goes through the same suffix logic on its
    // module part.
    assert_eq!(
        resolve(
            ws.path(),
            "cargo test -p trading --lib data_store_ahdm_tests::pine::parses_bars"
        ),
        pine
    );
    // `mid::leaf` → `src/top/mid/leaf.rs` + `src/top/mid/leaf/`.
    let ws = package_with(&["top.rs", "top/mid.rs", "top/mid/leaf.rs"]);
    let leaf = target(
        "crates/trading/src/top/mid/leaf.rs",
        "crates/trading/src/top/mid/leaf",
    );
    assert_eq!(
        resolve(ws.path(), "cargo test -p trading --lib mid::leaf"),
        leaf
    );
    // `leaf` alone, when unique.
    assert_eq!(resolve(ws.path(), "cargo test -p trading --lib leaf"), leaf);
    // A `mod.rs` leaf and a `_tests.rs` sibling read as `::tests`.
    let ws = package_with(&[
        "top.rs",
        "top/mid.rs",
        "top/mid/leaf/mod.rs",
        "top/mid_tests.rs",
    ]);
    assert_eq!(
        resolve(ws.path(), "cargo test -p trading --lib mid::leaf"),
        target(
            "crates/trading/src/top/mid/leaf/mod.rs",
            "crates/trading/src/top/mid/leaf"
        )
    );
    assert_eq!(
        resolve(
            ws.path(),
            "cargo test -p trading --lib mid::tests::round_trips"
        ),
        target(
            "crates/trading/src/top/mid_tests.rs",
            "crates/trading/src/top/mid_tests"
        )
    );
}

#[test]
fn a_full_suffix_hit_beats_a_shallower_exact_parent_and_an_exact_parent_beats_a_same_depth_stray() {
    // `src/mid.rs` exists but has no `leaf`; `top/mid/leaf.rs` matches every
    // segment, so the filter is that module's.
    let ws = package_with(&["mid.rs", "top.rs", "top/mid.rs", "top/mid/leaf.rs"]);
    assert_eq!(
        resolve(ws.path(), "cargo test -p trading --lib mid::leaf"),
        target(
            "crates/trading/src/top/mid/leaf.rs",
            "crates/trading/src/top/mid/leaf"
        )
    );
    // `store::strategy_spec` names a leaf that does not exist yet: the
    // exact parent `src/store.rs` wins over `src/other/store.rs`, which
    // only ties with it at the same depth.
    let ws = package_with(&["store.rs", "other.rs", "other/store.rs"]);
    assert_eq!(
        resolve(
            ws.path(),
            "cargo test -p trading --lib store::strategy_spec"
        ),
        target("crates/trading/src/store.rs", "crates/trading/src/store")
    );
}

#[test]
fn an_ambiguous_suffix_resolves_nothing_and_is_reported() {
    let ws = package_with(&["alpha.rs", "alpha/leaf.rs", "beta.rs", "beta/leaf.rs"]);
    let command = "cargo test -p trading --lib leaf";
    assert_eq!(resolve(ws.path(), command), None);
    assert_eq!(
        resolution(ws.path(), command),
        Resolution::Ambiguous {
            filter: "leaf".into(),
            candidates: vec![
                "crates/trading/src/alpha/leaf.rs".into(),
                "crates/trading/src/beta/leaf.rs".into(),
            ],
        }
    );
    // Narrowed by one segment, it resolves.
    assert_eq!(
        resolve(ws.path(), "cargo test -p trading --lib beta::leaf"),
        target(
            "crates/trading/src/beta/leaf.rs",
            "crates/trading/src/beta/leaf"
        )
    );
    // A bare `tests` never suffix-matches every `_tests.rs` in the tree.
    let ws = package_with(&["alpha.rs", "alpha_tests.rs"]);
    assert_eq!(
        resolve(ws.path(), "cargo test -p trading --lib tests"),
        None
    );
    assert_eq!(
        resolve(ws.path(), "cargo test -p trading --lib tests::smoke"),
        None
    );

    // The ambiguity is carried to the coder and the result.
    let ws = package_with(&["alpha.rs", "alpha/leaf.rs", "beta.rs", "beta/leaf.rs"]);
    let widened = widenable(
        ws.path(),
        None,
        &[],
        &[],
        &[
            command.to_string(),
            "cargo test -p trading --lib beta::leaf".to_string(),
        ],
    );
    let ambiguous =
        "leaf (crates/trading/src/alpha/leaf.rs, crates/trading/src/beta/leaf.rs)".to_string();
    assert_eq!(
        widened,
        Widenable {
            files: vec!["crates/trading/src/beta/leaf.rs".into()],
            dirs: vec!["crates/trading/src/beta/leaf".into()],
            ambiguous: vec![ambiguous.clone()],
        }
    );
    let targets = FocusedTestTargets {
        widened: Vec::new(),
        ambiguous: vec![ambiguous.clone()],
    };
    assert_eq!(
        preamble(&targets),
        format!(
            "\nFocused-test filters that matched several modules and widened nothing — narrow the \
             filter to the module you mean: {ambiguous}.\n"
        )
    );
    let mut result = crate::WorkflowV2Result::default();
    stamp_result(&mut result, &targets);
    assert_eq!(
        result.data["focused_test_targets_widened"],
        serde_json::Value::Null
    );
    assert_eq!(
        result.data["focused_test_targets_ambiguous"],
        serde_json::json!([ambiguous])
    );
    assert_eq!(preamble(&FocusedTestTargets::default()), "");
}
