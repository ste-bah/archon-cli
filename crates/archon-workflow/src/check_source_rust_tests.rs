use super::*;

const FILE: &str = "#![cfg(feature = \"x\")]\n//! Doc.\nuse x::y;\n\npub fn run() -> u32 {\n    1\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    /// Doc.\n    #[test]\n    #[ignore]\n    fn works() {\n        assert_eq!(run(), 1);\n    }\n\n    #[tokio::test]\n    async fn works_async() {}\n\n    mod inner {\n        #[test]\n        fn works() {}\n    }\n}\n";

#[test]
fn items_are_keyed_by_their_module_path_so_a_same_named_test_cannot_shift_them() {
    let keys: Vec<String> = keyed_tests(FILE).into_iter().map(|i| i.key).collect();
    assert_eq!(
        keys,
        [
            "fn:tests::works",
            "fn:tests::works_async",
            "fn:tests::inner::works"
        ]
    );
    let text = item_text(FILE, "fn:tests::works").unwrap();
    assert!(text.starts_with("#[test]\n    #[ignore]"), "{text}");
    assert!(text.ends_with('}'), "{text}");
    assert!(
        !items(FILE)
            .iter()
            .find(|i| i.name == "run")
            .unwrap()
            .is_test
    );
    // A same-named test added earlier elsewhere leaves the key's text alone.
    let added = FILE.replace("pub fn run()", "#[test]\nfn works() {}\n\npub fn run()");
    assert_eq!(item_text(&added, "fn:tests::works").unwrap(), text);
    assert_eq!(key_name("fn:tests::inner::works#1"), "works");
    assert_eq!(
        enclosing_mods("fn:tests::inner::works"),
        ["mod:tests", "mod:tests::inner"]
    );
}

#[test]
fn module_headers_and_inner_cfgs_are_items_too() {
    assert_eq!(
        item_text(FILE, "mod:tests").unwrap(),
        "#[cfg(test)]\nmod tests "
    );
    assert_eq!(
        item_text(FILE, CFG_KEY).unwrap(),
        "#![cfg(feature = \"x\")]"
    );
    // Switching the module off is undone by splicing its header back.
    let off = FILE.replace("#[cfg(test)]\nmod tests", "#[cfg(any())]\nmod tests");
    let back = splice_item(&off, "mod:tests", item_text(FILE, "mod:tests").as_deref());
    assert_eq!(back.unwrap(), FILE);
    let uncfg = splice_item(FILE, CFG_KEY, None).unwrap();
    assert!(!uncfg.contains("#![cfg"));
    assert_eq!(
        splice_item(&uncfg, CFG_KEY, Some("#![cfg(feature = \"x\")]")).unwrap(),
        FILE
    );
}

#[test]
fn a_filter_selects_by_substring_or_exact_name() {
    assert_eq!(matching_tests(FILE, "work", false).len(), 3);
    assert_eq!(matching_tests(FILE, "works", true).len(), 2);
    assert!(matching_tests(FILE, "missing", false).is_empty());
}

#[test]
fn splicing_replaces_or_removes_one_item_and_keeps_the_rest() {
    let weakened = FILE.replace("assert_eq!(run(), 1);", "");
    let pinned = item_text(FILE, "fn:tests::works").unwrap();
    let restored = splice_item(&weakened, "fn:tests::works", Some(&pinned)).unwrap();
    assert_eq!(restored, FILE);
    let removed = splice_item(FILE, "fn:tests::works_async", None).unwrap();
    assert!(!removed.contains("works_async"));
    assert!(removed.contains("fn works()"));
    assert!(splice_item(FILE, "fn:nope", None).is_none());
}

#[test]
fn module_files_follow_rust_rules_and_name_their_declaration() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("tests/common")).unwrap();
    std::fs::write(root.join("tests/common/mod.rs"), "").unwrap();
    let top =
        "mod common;\n#[path = \"shared/x.rs\"]\nmod x;\nmod absent;\nmod inline { mod deep; }\n";
    let children: Vec<(String, String)> = mod_children(root, "tests/it.rs", top, true)
        .into_iter()
        .map(|c| (c.path, c.decl))
        .collect();
    let expect = |p: &str, d: &str| (p.to_string(), d.to_string());
    assert_eq!(
        children,
        [
            expect("tests/absent.rs", "mod:absent"),
            expect("tests/common/mod.rs", "mod:common"),
            expect("tests/inline/deep.rs", "mod:inline::deep"),
            expect("tests/shared/x.rs", "mod:x"),
        ]
    );
    let nested = mod_children(root, "src/a/b.rs", "mod c;", false);
    assert_eq!(nested[0].path, "src/a/b/c.rs");
}

/// Review minor 5: a `#[path]` climbing above the tree is refused, never
/// folded into a path inside it.
#[test]
fn a_module_path_leaving_the_tree_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let children = mod_children(
        dir.path(),
        "src/lib.rs",
        "#[path = \"../../outside.rs\"]\nmod outside;\n#[path = \"../shared/x.rs\"]\nmod x;\n",
        true,
    );
    let outside = children.iter().find(|c| c.decl == "mod:outside").unwrap();
    assert!(outside.escapes, "{outside:?}");
    let inside = children.iter().find(|c| c.decl == "mod:x").unwrap();
    assert!(!inside.escapes);
    assert_eq!(inside.path, "shared/x.rs");
}
