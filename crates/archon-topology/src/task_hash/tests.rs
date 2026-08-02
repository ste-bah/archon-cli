use super::*;

#[test]
fn task_hash_is_stable_across_equivalent_work() {
    // Same work, different run, different absolute checkout, different issue
    // number, different wording order. The corpus must see one bucket.
    let first = task_hash(
        "Fix the panic in the YAML parser at C:\\repo\\crates\\archon-workflow\\src/spec.rs \
         (run wf-6f1c2d3e4a5b, issue 4471, 2026-08-02T11:04:00Z)",
    );
    let second = task_hash(
        "the YAML parser panic, fix it — /home/dev/checkout/crates/archon-workflow/src/spec.rs \
         run wf-91ab77cc0012, issue 5093, 2026-08-03T02:19:00Z",
    );

    assert_eq!(first, second, "equivalent work must share a task_hash");
}

#[test]
fn task_hash_is_pure() {
    let text = "refactor the write coordinator";
    assert_eq!(task_hash(text), task_hash(text));
}

#[test]
fn task_hash_differs_across_task_classes() {
    // Identical prose body, forced into each class. Nothing may collide: the
    // class is hashed as well as prefixed precisely so this holds.
    let body = "the write coordinator module";
    let hashes: Vec<String> = TaskClass::all()
        .into_iter()
        .map(|class| task_hash_for_class(class, body))
        .collect();

    for (i, left) in hashes.iter().enumerate() {
        for right in hashes.iter().skip(i + 1) {
            assert_ne!(left, right, "task classes must not collide");
        }
    }
}

#[test]
fn task_hash_differs_across_distinct_work() {
    assert_ne!(
        task_hash("refactor the write coordinator"),
        task_hash("refactor the provider auth cache"),
    );
}

#[test]
fn task_hash_carries_its_class_as_a_visible_prefix() {
    let hash = task_hash("audit the orchestrator for unwired paths");
    assert!(
        hash.starts_with("review:"),
        "expected a review-classed hash, got {hash}"
    );
    let (_, hex) = hash.split_once(':').expect("hash must be class-prefixed");
    assert_eq!(hex.len(), 16);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn classification_covers_every_class() {
    assert_eq!(
        classify_task("refactor the dispatcher, extract the retry loop"),
        TaskClass::Refactor
    );
    assert_eq!(
        classify_task("reproduce the crash and fix the deadlock"),
        TaskClass::BugHunt
    );
    assert_eq!(
        classify_task("migrate the schema and bump the dependency"),
        TaskClass::Migration
    );
    assert_eq!(
        classify_task("review and audit the permission layer"),
        TaskClass::Review
    );
    assert_eq!(
        classify_task("implement a new topology store"),
        TaskClass::Greenfield
    );
}

#[test]
fn classification_falls_back_to_greenfield_on_no_signal() {
    assert_eq!(classify_task("the widget"), TaskClass::Greenfield);
    assert_eq!(classify_task(""), TaskClass::Greenfield);
}

#[test]
fn classification_scores_rather_than_first_matches() {
    // One migration marker, two bug-hunt markers. A first-match rule keyed on
    // table order would answer Refactor or Migration depending on declaration
    // order; scoring answers BugHunt.
    assert_eq!(
        classify_task("the migration crashes, reproduce the panic"),
        TaskClass::BugHunt
    );
}

#[test]
fn classification_ties_break_deterministically() {
    // One marker each for Refactor and BugHunt. Refactor is the earlier
    // variant, so it wins, and it must win every time.
    let text = "refactor the crash handler";
    let first = classify_task(text);
    assert_eq!(first, TaskClass::Refactor);
    for _ in 0..16 {
        assert_eq!(classify_task(text), first);
    }
}

#[test]
fn normalization_strips_volatile_identifiers() {
    let tokens = normalize_task_text(
        "run wf-6f1c2d3e4a5b for 550e8400-e29b-41d4-a716-446655440000 on 2026-08-02 \
         touching /abs/path/parser.rs at commit deadbeefcafe1234 line 91422 count 7",
    );

    for volatile in [
        "wf-6f1c2d3e4a5b",
        "550e8400",
        "e29b",
        "446655440000",
        "2026",
        "abs",
        "path",
        "deadbeefcafe1234",
        "91422",
    ] {
        assert!(
            !tokens.iter().any(|token| token == volatile),
            "volatile token {volatile} survived normalization: {tokens:?}",
        );
    }
    assert!(
        tokens.iter().any(|token| token == "parser"),
        "the path basename must survive: {tokens:?}"
    );
}

#[test]
fn normalization_keeps_identifier_prefixed_words() {
    // `run-tests` is a word, not a run id. The suffix must look like an
    // identifier before the prefix rule fires.
    let tokens = normalize_task_text("run-tests and run-4471abc");
    assert!(tokens.iter().any(|token| token == "run"));
    assert!(tokens.iter().any(|token| token == "tests"));
    assert!(
        !tokens.iter().any(|token| token.contains("4471abc")),
        "{tokens:?}"
    );
}

#[test]
fn normalization_is_order_insensitive_and_deduplicated() {
    let a = normalize_task_text("parser crash parser fix");
    let b = normalize_task_text("fix crash parser");
    assert_eq!(a, b);
    assert_eq!(a.len(), 3, "{a:?}");
}

#[test]
fn normalization_erases_path_flavour() {
    assert_eq!(
        normalize_task_text("C:\\work\\repo\\src\\lib.rs"),
        normalize_task_text("/var/tmp/other/src/lib.rs"),
    );
}

#[test]
fn class_wire_form_round_trips() {
    for class in TaskClass::all() {
        assert_eq!(TaskClass::parse(class.as_str()), Some(class));
    }
    assert_eq!(TaskClass::parse("nonsense"), None);
}

#[test]
fn fnv1a_matches_the_published_vectors() {
    // Guards the corpus key against an accidental edit to the constants.
    let mut hasher = Fnv1a::new();
    assert_eq!(hasher.finish(), 0xcbf2_9ce4_8422_2325);
    hasher.write(b"a");
    assert_eq!(hasher.finish(), 0xaf63_dc4c_8601_ec8c);

    let mut hasher = Fnv1a::new();
    hasher.write(b"foobar");
    assert_eq!(hasher.finish(), 0x85944171f73967e8);
}
