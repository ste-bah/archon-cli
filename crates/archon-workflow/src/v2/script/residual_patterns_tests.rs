//! Issue-117: patterns resolve to the exact files the tree holds, bounded.

use super::*;

fn files(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|f| f.to_string()).collect()
}

fn tree() -> BTreeSet<String> {
    files(&[
        "crates/t/src/providers/tradingview_store.rs",
        "crates/t/src/providers/stooq_store.rs",
        "crates/t/src/providers/mod.rs",
        "crates/t/src/data_store/ingest.rs",
        "crates/u/src/lib.rs",
        "crates/t/src/lib.rs",
    ])
}

fn named(text: &str) -> Vec<String> {
    resolve_named(text, Path::new("/repo"), &tree())
}

#[test]
fn globs_braces_short_forms_and_directories_resolve_to_exact_files() {
    assert_eq!(
        named("the providers/*_store.rs lanes are unowned"),
        [
            "crates/t/src/providers/stooq_store.rs",
            "crates/t/src/providers/tradingview_store.rs"
        ]
    );
    assert_eq!(
        named("crates/t/src/providers/{tradingview,stooq,absent}_store.rs are declared by no task"),
        [
            "crates/t/src/providers/stooq_store.rs",
            "crates/t/src/providers/tradingview_store.rs"
        ]
    );
    assert_eq!(
        named("see data_store/ingest.rs:12 and `providers/mod.rs`"),
        [
            "crates/t/src/data_store/ingest.rs",
            "crates/t/src/providers/mod.rs"
        ]
    );
    assert_eq!(named("the crates/t/src/providers/ lane").len(), 3);
    assert!(
        named("src/lib.rs is ambiguous").is_empty(),
        "two files end with it"
    );
    assert_eq!(named("/repo/crates/u/src/lib.rs"), ["crates/u/src/lib.rs"]);
    assert!(named("../crates/u/src/lib.rs and crates/u/src/nope.rs").is_empty());
}

#[test]
fn a_pattern_wider_than_the_cap_names_nothing() {
    let wide: BTreeSet<String> = (0..=PATTERN_CAP)
        .map(|n| format!("crates/w/src/f{n}.rs"))
        .collect();
    let root = Path::new("/repo");
    assert!(resolve_named("crates/w/src/*.rs", root, &wide).is_empty());
    assert!(resolve_named("crates/w/src/", root, &wide).is_empty());
    assert_eq!(
        resolve_named("crates/w/src/f3.rs", root, &wide),
        ["crates/w/src/f3.rs"]
    );
}

#[test]
fn double_star_crosses_segments_and_single_star_does_not() {
    assert!(glob(
        "crates/**/stooq_store.rs",
        "crates/t/src/providers/stooq_store.rs"
    ));
    assert!(glob("crates/*/src/lib.rs", "crates/u/src/lib.rs"));
    assert!(!glob("crates/*/lib.rs", "crates/u/src/lib.rs"));
    assert!(glob("a/**/b.rs", "a/b.rs"));
}
