use super::*;
use crate::graph::MemoryGraph;

// -- should_extract -------------------------------------------------

#[test]
fn should_extract_fires_at_interval() {
    let config = ExtractionConfig {
        interval: 5,
        enabled: true,
        min_turns_between: 1,
    };
    let mut state = ExtractionState::default();
    // Simulate 4 turns — not yet.
    for _ in 0..4 {
        state.record_turn();
    }
    assert!(!should_extract(&config, &state, 4));

    // 5th turn — should fire.
    state.record_turn();
    assert!(should_extract(&config, &state, 5));
}

#[test]
fn should_extract_respects_min_turns_between() {
    let config = ExtractionConfig {
        interval: 1,
        enabled: true,
        min_turns_between: 3,
    };
    let mut state = ExtractionState::default();
    state.record_turn();
    // Last extraction was at turn 0, current turn is 1 — only 1 elapsed.
    assert!(!should_extract(&config, &state, 1));

    // current_turn = 3 — 3 turns since last_extraction_turn 0
    assert!(should_extract(&config, &state, 3));
}

#[test]
fn should_extract_disabled() {
    let config = ExtractionConfig {
        interval: 1,
        enabled: false,
        min_turns_between: 0,
    };
    let mut state = ExtractionState::default();
    for _ in 0..10 {
        state.record_turn();
    }
    assert!(!should_extract(&config, &state, 10));
}

// -- parse_extraction_response --------------------------------------

#[test]
fn parse_valid_json() {
    let json = r#"[
        {"content": "User prefers dark mode", "memory_type": "preference", "tags": ["ui"]},
        {"content": "Project uses Rust 2024 edition", "memory_type": "fact", "tags": ["rust", "config"]}
    ]"#;
    let mems = parse_extraction_response(json).expect("should parse");
    assert_eq!(mems.len(), 2);
    assert_eq!(mems[0].memory_type, MemoryType::Preference);
    assert_eq!(mems[0].tags, vec!["ui"]);
    assert_eq!(mems[1].memory_type, MemoryType::Fact);
}

#[test]
fn parse_invalid_json_no_crash() {
    let bad = "this is not json at all {{{}}}";
    let mems = parse_extraction_response(bad).expect("should not error");
    assert!(mems.is_empty());
}

#[test]
fn parse_markdown_fenced_json() {
    let fenced = r#"```json
[{"content":"a rule","memory_type":"rule","tags":[]}]
```"#;
    let mems = parse_extraction_response(fenced).expect("should parse");
    assert_eq!(mems.len(), 1);
    assert_eq!(mems[0].memory_type, MemoryType::Rule);
}

#[test]
fn parse_skips_unknown_types_and_empty_content() {
    let json = r#"[
        {"content":"good","memory_type":"fact","tags":[]},
        {"content":"","memory_type":"fact","tags":[]},
        {"content":"alien","memory_type":"unknown_type","tags":[]}
    ]"#;
    let mems = parse_extraction_response(json).expect("should parse");
    assert_eq!(mems.len(), 1);
    assert_eq!(mems[0].content, "good");
}

// -- build_extraction_prompt ----------------------------------------

#[test]
fn build_prompt_includes_messages() {
    let msgs = vec!["Hello".to_string(), "How are you?".to_string()];
    let prompt = build_extraction_prompt(&msgs, &[]);
    assert!(prompt.contains("Hello"));
    assert!(prompt.contains("How are you?"));
    assert!(prompt.contains("JSON"));
}

/// The prompt must state whose behaviour a rule constrains.
///
/// Not decoration: a `rule` goes into every later system prompt unreviewed,
/// and the version of this prompt that omitted this produced
/// `Avoid: <the thing the user asked for>` from correction turns -- inverting
/// the user's intent. The ingest caps cannot catch that, because an inverted
/// rule is short.
#[test]
fn prompt_directs_rules_at_the_assistant_not_the_user() {
    let prompt = build_extraction_prompt(&["anything".to_string()], &[]);

    assert!(
        prompt.contains("ASSISTANT"),
        "the prompt must say whose behaviour a rule describes"
    );
    assert!(
        prompt.contains("never what\nthe user should do"),
        "the prompt must rule out writing the user's instructions back as rules"
    );
    assert!(
        prompt.contains("never \"Avoid running the tests before pushing\""),
        "the prompt must show the inversion it is guarding against"
    );
}

/// The stated limits must be the enforced limits.
///
/// If they drift apart the model is asked for content that `parse_extraction_response`
/// then silently discards -- a wasted call and a lost memory, with nothing in
/// the output to explain it.
#[test]
fn prompt_states_the_limits_that_are_actually_enforced() {
    let prompt = build_extraction_prompt(&["anything".to_string()], &[]);

    assert!(
        prompt.contains(&MAX_EXTRACTED_CONTENT_CHARS.to_string()),
        "the general cap must be stated to the model"
    );
    assert!(
        prompt.contains(&MAX_RULE_CONTENT_CHARS.to_string()),
        "the rule cap must be stated to the model"
    );
    assert!(
        prompt.contains("Never copy a\ndocument"),
        "the prompt must tell the model not to paste documents back"
    );
}

// -- store_extracted ------------------------------------------------

#[test]
fn store_with_tags() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let mems = vec![ExtractedMemory {
        content: "Rust edition is 2024".into(),
        memory_type: MemoryType::Fact,
        tags: vec!["rust".into()],
    }];
    let stored = store_extracted(&graph, &mems, "sess-001").expect("store");
    assert_eq!(stored, 1);

    let results = graph.recall_memories("Rust edition", 10).expect("recall");
    assert_eq!(results.len(), 1);
    assert!(results[0].tags.contains(&"auto-extract".to_string()));
    assert!(results[0].tags.contains(&"session:sess-001".to_string()));
    assert!(results[0].tags.contains(&"rust".to_string()));
    assert_eq!(results[0].source_type, "auto-extract");
}

#[test]
fn dedup_skips_substring_match() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph");

    // Pre-populate with an existing memory.
    graph
        .store_memory(
            "User prefers dark mode in all editors",
            "",
            MemoryType::Preference,
            0.5,
            &[],
            "manual",
            "",
        )
        .expect("seed");

    // Try to store a substring of the existing memory.
    let mems = vec![ExtractedMemory {
        content: "dark mode in all editors".into(),
        memory_type: MemoryType::Preference,
        tags: vec![],
    }];
    let stored = store_extracted(&graph, &mems, "s1").expect("store");
    assert_eq!(stored, 0, "duplicate should be skipped");
}

/// A whitespace-only restatement must not create a second copy.
///
/// This is the case the containment check cannot catch -- neither string
/// contains the other once the spacing differs -- and it is how the same
/// document accumulated copies. The fingerprint normalises whitespace and
/// case, so it sees them as one.
#[test]
fn dedup_catches_a_respaced_restatement_that_containment_misses() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let original = "Rust edition must be 2024";
    let respaced = "Rust  edition   must be 2024";

    assert!(
        !original.to_lowercase().contains(&respaced.to_lowercase())
            && !respaced.to_lowercase().contains(&original.to_lowercase()),
        "fixture must not be catchable by the containment check, or it proves nothing"
    );

    let first = store_extracted(
        &graph,
        &[ExtractedMemory {
            content: original.into(),
            memory_type: MemoryType::Rule,
            tags: vec![],
        }],
        "s1",
    )
    .expect("store original");
    assert_eq!(first, 1);

    let second = store_extracted(
        &graph,
        &[ExtractedMemory {
            content: respaced.into(),
            memory_type: MemoryType::Rule,
            tags: vec![],
        }],
        "s2",
    )
    .expect("store restatement");

    assert_eq!(second, 0, "a respaced restatement must not be stored again");
}

/// An oversized rule is discarded rather than stored.
///
/// Rules are rendered into the system prompt unconditionally, so an
/// oversized one is paid for on every request. This is the check that stops
/// a pasted document becoming a permanent behavioural rule.
#[test]
fn parse_discards_a_rule_longer_than_the_rule_cap() {
    let document = "x".repeat(MAX_RULE_CONTENT_CHARS + 1);
    let json = serde_json::to_string(&serde_json::json!([
        { "content": document, "memory_type": "rule", "tags": [] },
        { "content": "keep this one", "memory_type": "rule", "tags": [] },
    ]))
    .expect("fixture json");

    let parsed = parse_extraction_response(&json).expect("parse");

    assert_eq!(parsed.len(), 1, "the oversized rule must be dropped");
    assert_eq!(parsed[0].content, "keep this one");
}

/// The general cap is looser than the rule cap but still bounded, so a
/// pasted document cannot enter as a `fact` either.
#[test]
fn parse_discards_any_memory_longer_than_the_general_cap() {
    let document = "y".repeat(MAX_EXTRACTED_CONTENT_CHARS + 1);
    let json = serde_json::to_string(&serde_json::json!([
        { "content": document, "memory_type": "fact", "tags": [] },
    ]))
    .expect("fixture json");

    assert!(
        parse_extraction_response(&json).expect("parse").is_empty(),
        "an oversized fact must be dropped"
    );

    // A rule-length fact is still fine: the general cap must not be so
    // tight that ordinary memories are lost.
    let ordinary = "z".repeat(MAX_RULE_CONTENT_CHARS + 1);
    let json = serde_json::to_string(&serde_json::json!([
        { "content": ordinary, "memory_type": "fact", "tags": [] },
    ]))
    .expect("fixture json");
    assert_eq!(parse_extraction_response(&json).expect("parse").len(), 1);
}

#[test]
fn fingerprint_is_stable_and_content_sensitive() {
    // Stability matters across builds, not just within one run: a seeded
    // hash would silently stop matching rows written by an older binary.
    assert_eq!(content_hash_tag("same text"), content_hash_tag("same text"));
    assert_eq!(
        content_hash_tag(" Same   Text "),
        content_hash_tag("same text")
    );
    assert_ne!(
        content_hash_tag("one thing"),
        content_hash_tag("another thing")
    );
}

// -- extraction state tracking --------------------------------------

#[test]
fn extraction_state_tracking() {
    let mut state = ExtractionState::default();
    assert_eq!(state.turns_since_last_extraction, 0);
    assert_eq!(state.last_extraction_turn, 0);

    state.record_turn();
    state.record_turn();
    assert_eq!(state.turns_since_last_extraction, 2);

    state.record_extraction(7);
    assert_eq!(state.turns_since_last_extraction, 0);
    assert_eq!(state.last_extraction_turn, 7);

    state.record_turn();
    assert_eq!(state.turns_since_last_extraction, 1);
}
