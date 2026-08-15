use super::*;

#[test]
fn expands_declared_file_backed_modules_from_sibling_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/foo")).expect("module dir");
    fs::write(repo.join("src/foo.rs"), "mod bar;\npub mod baz;\n").expect("foo");
    fs::write(repo.join("src/foo/bar.rs"), "").expect("bar");
    fs::write(repo.join("src/foo/baz.rs"), "").expect("baz");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/foo.rs".to_string()],
        Some(&repo.display().to_string()),
    )
    .expect("expanded");

    assert_eq!(expanded.declared_target_files, vec!["src/foo.rs"]);
    assert_eq!(
        expanded.target_files,
        vec!["src/foo.rs", "src/foo/bar.rs", "src/foo/baz.rs"]
    );
    assert_eq!(expanded.target_dir_scopes, vec!["src/foo"]);
    assert_eq!(expanded.target_file_expansions[0].source, "src/foo.rs");
}

#[test]
fn module_directory_expansion_allows_new_child_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/data_store")).expect("module dir");
    fs::write(repo.join("src/data_store.rs"), "mod io;\n").expect("module");
    fs::write(repo.join("src/data_store/io.rs"), "").expect("child");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/data_store.rs".to_string()],
        Some(&repo.display().to_string()),
    )
    .expect("expanded");

    assert!(
        !expanded
            .target_files
            .contains(&"src/data_store".to_string())
    );
    assert!(
        expanded
            .target_dir_scopes
            .contains(&"src/data_store".to_string())
    );
}

/// `#[cfg(test)]` and `#[path]` are written in either order in this
/// repository. Both must resolve, or whichever order is unhandled silently
/// loses ownership of the file.
#[test]
fn a_path_attribute_survives_a_neighbouring_attribute() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/compression")).expect("dir");
    fs::write(repo.join("src/compression/tests.rs"), "").expect("tests");

    for source in [
        "#[cfg(test)]\n#[path = \"compression/tests.rs\"]\nmod tests;\n",
        "#[path = \"compression/tests.rs\"]\n#[cfg(test)]\nmod tests;\n",
    ] {
        fs::write(repo.join("src/compression.rs"), source).expect("declaring file");
        let expanded = expand_declared_rust_module_targets(
            "item",
            &["src/compression.rs".to_string()],
            repo.to_str(),
        )
        .expect("expansion");
        assert!(
            expanded
                .target_files
                .contains(&"src/compression/tests.rs".to_string()),
            "attribute order must not change ownership: {source:?} -> {:?}",
            expanded.target_files
        );
    }
}

/// The live failure after the `#[path]` fix. `data_lake.rs` declares
/// `mod tests;`, and `data_lake/tests.rs` declares `mod
/// artifact_tolerance;`. One pass found the child and stopped, so the
/// grandchild was unowned and its branch failed write-scope.
#[test]
fn ownership_reaches_a_grandchild_module() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/data_lake/tests")).expect("dirs");
    fs::write(repo.join("src/data_lake.rs"), "mod tests;\n").expect("root");
    fs::write(
        repo.join("src/data_lake/tests.rs"),
        "mod artifact_tolerance;\n",
    )
    .expect("child");
    fs::write(repo.join("src/data_lake/tests/artifact_tolerance.rs"), "").expect("grandchild");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/data_lake.rs".to_string()],
        repo.to_str(),
    )
    .expect("expansion");

    assert!(
        expanded
            .target_files
            .contains(&"src/data_lake/tests.rs".to_string())
    );
    assert!(
        expanded
            .target_files
            .contains(&"src/data_lake/tests/artifact_tolerance.rs".to_string()),
        "a grandchild module must be owned: {:?}",
        expanded.target_files
    );
}

/// Transitive walking must terminate even if two files declare each other.
#[test]
fn a_module_cycle_terminates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/a")).expect("dir a");
    fs::create_dir_all(repo.join("src/a/b")).expect("dir b");
    fs::write(repo.join("src/a.rs"), "mod b;\n").expect("a");
    // `a/b.rs` points back at `a.rs` through an explicit path.
    fs::write(repo.join("src/a/b.rs"), "#[path = \"../a.rs\"]\nmod a;\n").expect("b");

    let expanded =
        expand_declared_rust_module_targets("item", &["src/a.rs".to_string()], repo.to_str())
            .expect("expansion");

    assert!(expanded.target_files.contains(&"src/a/b.rs".to_string()));
}

/// The live failure. `ahdm_test_support.rs` is owned and splices its two
/// halves in with `include!`. Those halves are part of the owning file, but
/// they are not modules, so expansion never saw them and the branch that
/// edited them failed write-scope.
#[test]
fn an_included_file_is_owned_by_the_file_that_includes_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/data_store")).expect("dir");
    fs::write(
        repo.join("src/data_store/ahdm_test_support.rs"),
        "include!(\"ahdm_test_support_a.rs\");\ninclude!(\"ahdm_test_support_b.rs\");\n",
    )
    .expect("owner");
    fs::write(repo.join("src/data_store/ahdm_test_support_a.rs"), "").expect("a");
    fs::write(repo.join("src/data_store/ahdm_test_support_b.rs"), "").expect("b");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/data_store/ahdm_test_support.rs".to_string()],
        repo.to_str(),
    )
    .expect("expansion");

    for half in ["a", "b"] {
        assert!(
            expanded
                .target_files
                .contains(&format!("src/data_store/ahdm_test_support_{half}.rs")),
            "included half {half} must be owned: {:?}",
            expanded.target_files
        );
    }
}

/// An include may sit in a subdirectory, and the recursion must follow it.
#[test]
fn a_nested_include_is_followed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/world_model/root")).expect("dir");
    fs::write(
        repo.join("src/world_model.rs"),
        "include!(\"world_model/root/00_dispatch.rs\");\n",
    )
    .expect("owner");
    fs::write(
        repo.join("src/world_model/root/00_dispatch.rs"),
        "include!(\"01_helpers.rs\");\n",
    )
    .expect("dispatch");
    fs::write(repo.join("src/world_model/root/01_helpers.rs"), "").expect("helpers");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/world_model.rs".to_string()],
        repo.to_str(),
    )
    .expect("expansion");

    assert!(
        expanded
            .target_files
            .contains(&"src/world_model/root/00_dispatch.rs".to_string())
    );
    assert!(
        expanded
            .target_files
            .contains(&"src/world_model/root/01_helpers.rs".to_string()),
        "recursion must follow an include inside an include: {:?}",
        expanded.target_files
    );
}

/// A build-artefact include names no repository file and must not become one.
#[test]
fn a_generated_include_is_not_treated_as_a_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src")).expect("dir");
    fs::write(
        repo.join("src/generated.rs"),
        "include!(concat!(env!(\"OUT_DIR\"), \"/bindings.rs\"));\n",
    )
    .expect("owner");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/generated.rs".to_string()],
        repo.to_str(),
    )
    .expect("expansion");

    assert_eq!(expanded.target_files, vec!["src/generated.rs".to_string()]);
}

#[test]
fn inline_modules_do_not_invent_file_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/foo")).expect("module dir");
    fs::write(repo.join("src/foo.rs"), "mod inline {}\nmod missing;\n").expect("foo");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/foo.rs".to_string()],
        Some(&repo.display().to_string()),
    )
    .expect("expanded");

    assert_eq!(expanded.target_files, vec!["src/foo.rs"]);
    assert!(expanded.target_file_expansions[0].notes[0].contains("declared module 'missing'"));
}

#[test]
fn lib_and_main_are_not_broadly_expanded() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/sub")).expect("module dir");
    fs::write(repo.join("src/lib.rs"), "mod sub;\n").expect("lib");
    fs::write(repo.join("src/sub.rs"), "").expect("sub");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/lib.rs".to_string()],
        Some(&repo.display().to_string()),
    )
    .expect("expanded");

    assert_eq!(expanded.target_files, vec!["src/lib.rs"]);
    assert!(expanded.target_file_expansions.is_empty());
}

#[test]
fn unsafe_targets_still_reject() {
    let error =
        expand_declared_rust_module_targets("item", &["../outside.rs".to_string()], Some("/repo"))
            .expect_err("unsafe target");

    assert!(error.to_string().contains("unsafe"));
}

/// The live failure. `validation_tests.rs` declares its cases with
/// `#[path = "tests/…"]`, which puts them outside the module directory the
/// convention searches. Resolving by convention left them unowned, and the
/// branch that edited one lost on write-scope for touching its own file.
#[test]
fn a_module_moved_by_a_path_attribute_is_still_owned() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/data_store/tests")).expect("tests dir");
    fs::create_dir_all(repo.join("src/data_store/validation_tests")).expect("module dir");
    fs::write(
        repo.join("src/data_store/validation_tests.rs"),
        "#[path = \"tests/validation_atomicity.rs\"]\nmod validation_atomicity;\n\
         #[path = \"validation_tests/contract_core.rs\"]\nmod contract_core;\n",
    )
    .expect("declaring file");
    fs::write(
        repo.join("src/data_store/tests/validation_atomicity.rs"),
        "",
    )
    .expect("relocated");
    fs::write(
        repo.join("src/data_store/validation_tests/contract_core.rs"),
        "",
    )
    .expect("conventional");

    let expanded = expand_declared_rust_module_targets(
        "item",
        &["src/data_store/validation_tests.rs".to_string()],
        repo.to_str(),
    )
    .expect("expansion");

    assert!(
        expanded
            .target_files
            .contains(&"src/data_store/tests/validation_atomicity.rs".to_string()),
        "a #[path]-relocated module must be owned: {:?}",
        expanded.target_files
    );
    assert!(
        expanded
            .target_files
            .contains(&"src/data_store/validation_tests/contract_core.rs".to_string())
    );
    assert!(
        expanded
            .target_file_expansions
            .iter()
            .all(|expansion| expansion.notes.is_empty()),
        "no module should be reported unresolvable"
    );
}

/// An inline attribute is the same declaration written differently.
#[test]
fn an_inline_path_attribute_resolves_too() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/elsewhere")).expect("dir");
    fs::write(
        repo.join("src/foo.rs"),
        "#[path = \"elsewhere/bar.rs\"] mod bar;\n",
    )
    .expect("foo");
    fs::write(repo.join("src/elsewhere/bar.rs"), "").expect("bar");

    let expanded =
        expand_declared_rust_module_targets("item", &["src/foo.rs".to_string()], repo.to_str())
            .expect("expansion");

    assert!(
        expanded
            .target_files
            .contains(&"src/elsewhere/bar.rs".to_string()),
        "{:?}",
        expanded.target_files
    );
}

/// A `#[path]` must not leak onto an unrelated later declaration.
#[test]
fn a_path_attribute_applies_only_to_the_next_module() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path();
    fs::create_dir_all(repo.join("src/foo")).expect("dir");
    fs::write(
        repo.join("src/foo.rs"),
        "#[path = \"foo/moved.rs\"]\nmod moved;\nmod plain;\n",
    )
    .expect("foo");
    fs::write(repo.join("src/foo/moved.rs"), "").expect("moved");
    fs::write(repo.join("src/foo/plain.rs"), "").expect("plain");

    let expanded =
        expand_declared_rust_module_targets("item", &["src/foo.rs".to_string()], repo.to_str())
            .expect("expansion");

    assert!(
        expanded
            .target_files
            .contains(&"src/foo/moved.rs".to_string())
    );
    assert!(
        expanded
            .target_files
            .contains(&"src/foo/plain.rs".to_string()),
        "the second module resolves by convention, not by the earlier attribute: {:?}",
        expanded.target_files
    );
}
