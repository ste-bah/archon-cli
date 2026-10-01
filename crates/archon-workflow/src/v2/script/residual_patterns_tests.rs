//! Issue-117: patterns resolve to the exact files the tree holds, every one.

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

/// Batch O: a wide pattern names every file it matches (a cap used to make
/// it name nothing).
#[test]
fn a_wide_pattern_names_every_match() {
    let wide: BTreeSet<String> = (0..200).map(|n| format!("crates/w/src/f{n}.rs")).collect();
    let root = Path::new("/repo");
    assert_eq!(resolve_named("crates/w/src/*.rs", root, &wide).len(), 200);
    assert_eq!(resolve_named("crates/w/src/", root, &wide).len(), 200);
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

/// Batch O2 (CUT-12): a piece whose brace sets expand past the bound names
/// exactly the files its full expansion names -- exact paths, globs, short
/// forms and directories alike -- found without expanding it.
#[test]
fn a_brace_set_past_the_expansion_bound_still_names_every_file_it_names() {
    let mut tree = BTreeSet::new();
    for c in 0..5 {
        for m in 0..16 {
            for f in 0..16 {
                if (c + m + f) % 3 == 0 {
                    tree.insert(format!("crates/c{c}/src/m{m}/f{f}.rs"));
                }
            }
        }
    }
    let set = |prefix: &str, n: usize| {
        let items: Vec<String> = (0..n).map(|i| format!("{prefix}{i}")).collect();
        format!("{{{}}}", items.join(","))
    };
    let (c, m, f) = (set("c", 5), set("m", 16), set("f", 16));
    let pieces = [
        // exact paths
        format!("crates/{c}/src/{m}/{f}.rs"),
        // short forms (unique ones only), duplicated past the bound
        format!("src/{m}/{f}.rs{{,,,,}}"),
        // globs mixed with literal alternatives
        format!(
            "crates/{c}/src/{m}/{{f1*,f?,{}}}.rs",
            set("x", 14).trim_matches(['{', '}'])
        ),
        // directories
        format!(
            "crates/{c}/src/{m}/{{{},}}",
            set("z", 15).trim_matches(['{', '}'])
        ),
        // a location suffix
        format!("crates/{c}/src/{m}/{f}.rs:12"),
        // m1 edge cases: an alternative with `\\`, one that is `..`, and a
        // leading `-` -- each rejected per expansion, the rest still named
        format!(
            "crates/{c}/src/{m}/{{{},f\\1,..}}.rs",
            set("f", 14).trim_matches(['{', '}'])
        ),
        format!("{{-x,crates}}/c0/src/{m}/{f}.rs{{,,,,}}"),
        // m1: a location suffix or trailing punctuation INSIDE the last
        // brace set ends that expansion, and is trimmed with it
        format!("crates/{c}/src/{m}/{{,,,,,}}{{f0.rs:12,f3.rs.,f6.rs::lane,f9.rs}}"),
    ];
    // ... and a nested brace set, read as `braces` reads it (the first `}`
    // closes it), which names nothing here, past the bound or not.
    let nested = format!("crates/{{c0,c{{1,2}}}}/src/{m}/{f}.rs{{,,}}");
    let root = Path::new("/repo");
    for piece in &pieces {
        let expansions = full_expansion(piece);
        assert!(expansions.len() > BRACE_EXPANSION_BOUND, "{piece}");
        assert!(braces(piece).is_none(), "past the bound: {piece}");
        let mut expected = BTreeSet::new();
        for candidate in &expansions {
            if let Some((relative, directory)) = clean_candidate(candidate, root) {
                expected.extend(matches(&relative, directory, &tree));
            }
        }
        assert!(!expected.is_empty(), "{piece}");
        let named: BTreeSet<String> = resolve_named(piece, root, &tree).into_iter().collect();
        assert_eq!(named, expected, "{piece}");
    }
    assert!(full_expansion(&nested).len() > BRACE_EXPANSION_BOUND);
    let nested_expected: BTreeSet<String> = full_expansion(&nested)
        .iter()
        .filter_map(|candidate| clean_candidate(candidate, root))
        .flat_map(|(relative, directory)| matches(&relative, directory, &tree))
        .collect();
    let named: BTreeSet<String> = resolve_named(&nested, root, &tree).into_iter().collect();
    assert_eq!(named, nested_expected, "{nested}");
}

/// Every expansion, unbounded (the test's oracle).
fn full_expansion(piece: &str) -> Vec<String> {
    let Some(open) = piece.find('{') else {
        return vec![piece.to_string()];
    };
    let Some(close) = piece[open..].find('}').map(|at| open + at) else {
        return vec![piece.to_string()];
    };
    let (head, body, tail) = (&piece[..open], &piece[open + 1..close], &piece[close + 1..]);
    let rests = full_expansion(tail);
    body.split(',')
        .flat_map(|alternative| {
            rests
                .iter()
                .map(move |rest| format!("{head}{alternative}{rest}"))
        })
        .collect()
}
