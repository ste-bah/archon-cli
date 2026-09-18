//! Issue-48: the hub cap. A declaring file whose module fan-out is above
//! `HUB_MODULE_FAN_OUT_CAP` owns itself and its includes, not the tree.
use super::*;

/// A synthetic crate whose `hub/mod.rs` declares `count` file-backed modules
/// and splices in one `include!`d file.
fn hub_crate(repo: &Path, count: usize) {
    fs::create_dir_all(repo.join("src/hub")).expect("hub dir");
    let mut source = String::from("include!(\"spliced.rs\");\n");
    for index in 0..count {
        source.push_str(&format!("pub mod m{index};\n"));
        fs::write(repo.join(format!("src/hub/m{index}.rs")), "").expect("module");
    }
    fs::write(repo.join("src/hub/mod.rs"), source).expect("hub");
    fs::write(repo.join("src/hub/spliced.rs"), "").expect("spliced");
}

fn expand_hub(repo: &Path) -> ExpandedTargetFiles {
    expand_declared_rust_module_targets("item", &["src/hub/mod.rs".to_string()], repo.to_str())
        .expect("expansion")
}

/// Issue-48: a crate-level registry module declaring hundreds of modules made
/// its task own the whole tree. Above the cap the declaring file owns itself
/// and its includes only, grants no directory scope, and says why.
#[test]
fn a_hub_module_owns_itself_and_its_includes_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    hub_crate(temp.path(), 20);

    let expanded = expand_hub(temp.path());

    assert_eq!(
        expanded.target_files,
        vec!["src/hub/mod.rs", "src/hub/spliced.rs"],
        "a hub owns no declared module"
    );
    assert!(
        expanded.target_dir_scopes.is_empty(),
        "{:?}",
        expanded.target_dir_scopes
    );
    let note = &expanded.target_file_expansions[0].notes[0];
    assert!(
        note.starts_with("hub module: declares 20 file-backed modules")
            && note.contains("above the cap of 16")
            && note.ends_with("owning the declaring file only"),
        "{note}"
    );
}

/// A file declaring a handful of modules is a split, and is expanded as before.
#[test]
fn a_small_module_is_still_fully_expanded() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/small")).expect("dir");
    fs::write(repo.join("src/small.rs"), "mod a;\nmod b;\nmod c;\n").expect("small");
    for name in ["a", "b", "c"] {
        fs::write(repo.join(format!("src/small/{name}.rs")), "").expect("child");
    }

    let expanded =
        expand_declared_rust_module_targets("item", &["src/small.rs".to_string()], repo.to_str())
            .expect("expansion");

    assert_eq!(
        expanded.target_files,
        vec![
            "src/small.rs",
            "src/small/a.rs",
            "src/small/b.rs",
            "src/small/c.rs"
        ]
    );
    assert_eq!(expanded.target_dir_scopes, vec!["src/small"]);
    assert!(expanded.target_file_expansions[0].notes.is_empty());
}

/// The cap is inclusive: exactly the cap expands, one more is a hub.
#[test]
fn the_hub_cap_is_inclusive() {
    let at_cap = tempfile::tempdir().expect("tempdir");
    hub_crate(at_cap.path(), HUB_MODULE_FAN_OUT_CAP);
    let expanded = expand_hub(at_cap.path());
    assert_eq!(expanded.target_files.len(), HUB_MODULE_FAN_OUT_CAP + 2);
    assert_eq!(expanded.target_dir_scopes, vec!["src/hub"]);
    assert!(expanded.target_file_expansions[0].notes.is_empty());

    let above_cap = tempfile::tempdir().expect("tempdir");
    hub_crate(above_cap.path(), HUB_MODULE_FAN_OUT_CAP + 1);
    let expanded = expand_hub(above_cap.path());
    assert_eq!(
        expanded.target_files,
        vec!["src/hub/mod.rs", "src/hub/spliced.rs"]
    );
    assert!(expanded.target_dir_scopes.is_empty());
    assert!(expanded.target_file_expansions[0].notes[0].starts_with("hub module:"));
}

/// Ownership is transitive, so the cap counts grandchildren: a registry
/// whose one child fans out past the cap is a hub even though it declares one
/// module itself.
#[test]
fn the_hub_cap_counts_transitive_fan_out() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/hub/child")).expect("dirs");
    fs::write(repo.join("src/hub/mod.rs"), "mod child;\n").expect("hub");
    let mut child = String::new();
    for index in 0..HUB_MODULE_FAN_OUT_CAP {
        child.push_str(&format!("mod g{index};\n"));
        fs::write(repo.join(format!("src/hub/child/g{index}.rs")), "").expect("grandchild");
    }
    fs::write(repo.join("src/hub/child.rs"), child).expect("child");

    let expanded = expand_hub(repo);

    assert_eq!(
        expanded.target_files,
        vec!["src/hub/mod.rs"],
        "{:?}",
        expanded.target_files
    );
    assert!(expanded.target_dir_scopes.is_empty());
}

/// A `big.rs` whose modules live in `big/` is the split of one file, however
/// many pieces it split into (live, a declared parent carries 26 direct
/// children and its task must own them). The cap is for registries only.
#[test]
fn a_split_file_above_the_cap_still_owns_its_module_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/big")).expect("dir");
    let mut source = String::new();
    for index in 0..HUB_MODULE_FAN_OUT_CAP + 1 {
        source.push_str(&format!("mod m{index};\n"));
        fs::write(repo.join(format!("src/big/m{index}.rs")), "").expect("module");
    }
    fs::write(repo.join("src/big.rs"), source).expect("big");

    let expanded =
        expand_declared_rust_module_targets("item", &["src/big.rs".to_string()], repo.to_str())
            .expect("expansion");

    assert_eq!(expanded.target_files.len(), HUB_MODULE_FAN_OUT_CAP + 2);
    assert_eq!(expanded.target_dir_scopes, vec!["src/big"]);
    assert!(expanded.target_file_expansions[0].notes.is_empty());
}
