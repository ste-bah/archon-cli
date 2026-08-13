//! Explicit prompt-cache breakpoints on the OpenAI Chat Completions API.
//!
//! GPT-5.6 added `prompt_cache_breakpoint` on a content part, paired with
//! `prompt_cache_key` and an optional `prompt_cache_options` at the request
//! root. Three things about that shape cost money when they are wrong, so each
//! has a test here:
//!
//! * the fields must not reach a model that predates them — older models reject
//!   `prompt_cache_options` outright;
//! * `prompt_cache_options: {"mode": "explicit"}` turns OpenAI's *implicit*
//!   breakpoints off, so sending it uninvited can remove caching that was
//!   already happening for free;
//! * the breakpoint must close the stable head, not the whole system prompt,
//!   or it is rewritten every turn — and a cache write bills above plain input.

use archon_llm::cache_wire::{OPENAI_CACHE_DIRECTIVE_KEY, OpenAiCachePlacement, prompt_cache_key};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::OpenAiProvider;
use archon_llm::providers::openai::build_openai_request_body_cached;
use archon_llm::providers::openai_cache::supports_explicit_prompt_cache;

fn provider(base_url: Option<&str>) -> OpenAiProvider {
    OpenAiProvider::new(
        "test-key".to_string(),
        base_url.map(str::to_string),
        "gpt-5.6".to_string(),
    )
}

fn text_block(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "text", "text": text })
}

fn placement(stable: Option<usize>, explicit_only: bool) -> OpenAiCachePlacement {
    OpenAiCachePlacement {
        stable_system_blocks: stable,
        explicit_only,
        cache_key: "archon:test".to_string(),
    }
}

// ---------------------------------------------------------------------------
// The version gate
// ---------------------------------------------------------------------------

#[test]
fn only_gpt_5_6_and_later_take_explicit_breakpoints() {
    for model in ["gpt-5.6", "gpt-5.6-mini", "gpt-5.6-2026-01-15", "gpt-6"] {
        assert!(supports_explicit_prompt_cache(model), "{model}");
    }
    for model in [
        "gpt-5.5",
        "gpt-5",
        "gpt-4.1",
        "gpt-4o",
        "o3",
        "claude-opus-5",
    ] {
        assert!(
            !supports_explicit_prompt_cache(model),
            "{model} predates the field and would reject it"
        );
    }
}

#[test]
fn an_older_model_still_reports_automatic_caching() {
    let openai = provider(None);

    assert_eq!(
        openai.cache_strategy("gpt-5.5"),
        archon_llm::cache_strategy::CacheStrategy::Automatic,
        "GPT-5.5 caches implicitly — reporting None would show every request as \
         uncached in the cost figures"
    );
    assert!(matches!(
        openai.cache_strategy("gpt-5.6"),
        archon_llm::cache_strategy::CacheStrategy::ResponsesBreakpoints { .. }
    ));
}

/// `base_url` is overridable and the same struct is pointed at Azure and other
/// compatible hosts. Their caching is not OpenAI's to promise.
#[test]
fn a_non_openai_endpoint_gets_nothing_whatever_the_model_says() {
    let gateway = provider(Some("https://my-proxy.internal/v1"));

    assert_eq!(
        gateway.cache_strategy("gpt-5.6"),
        archon_llm::cache_strategy::CacheStrategy::None
    );
}

// ---------------------------------------------------------------------------
// The wire shape
// ---------------------------------------------------------------------------

#[test]
fn without_a_placement_the_body_is_byte_for_byte_what_it_always_was() {
    let system = vec![text_block("instructions")];
    let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];

    let body =
        build_openai_request_body_cached("gpt-5.6", 1024, &system, &messages, &[], false, None);

    assert_eq!(body["messages"][0]["content"], "instructions");
    assert!(body.get("prompt_cache_key").is_none());
    assert!(body.get("prompt_cache_options").is_none());
}

#[test]
fn a_placement_closes_the_stable_head_with_a_breakpoint() {
    let system = vec![
        text_block("stable one"),
        text_block("stable two"),
        text_block("volatile per-turn reminder"),
    ];
    let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];

    let body = build_openai_request_body_cached(
        "gpt-5.6",
        1024,
        &system,
        &messages,
        &[],
        false,
        Some(&placement(Some(2), false)),
    );

    let parts = body["messages"][0]["content"]
        .as_array()
        .expect("a cached system prompt is content parts, not a string");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["text"], "stable one\nstable two");
    assert_eq!(parts[0]["prompt_cache_breakpoint"]["mode"], "explicit");
    assert_eq!(
        parts[1]["text"], "volatile per-turn reminder",
        "the volatile tail must sit behind the breakpoint, not in front of it"
    );
    assert!(
        parts[1].get("prompt_cache_breakpoint").is_none(),
        "only one breakpoint belongs on the system prompt"
    );
}

#[test]
fn no_boundary_puts_the_breakpoint_after_the_whole_system_prompt() {
    let system = vec![text_block("all of it"), text_block("and this")];

    let body = build_openai_request_body_cached(
        "gpt-5.6",
        1024,
        &system,
        &[],
        &[],
        false,
        Some(&placement(None, false)),
    );

    let parts = body["messages"][0]["content"].as_array().expect("parts");
    assert_eq!(
        parts.len(),
        1,
        "nothing volatile follows, so nothing splits"
    );
    assert_eq!(parts[0]["prompt_cache_breakpoint"]["mode"], "explicit");
}

/// The asymmetry that decides the default: `explicit` is better than `hybrid`
/// only when the placement is right, and worse when it is not, because it
/// removes the caching OpenAI would have done unprompted.
#[test]
fn prompt_cache_options_is_sent_only_in_explicit_mode() {
    let system = vec![text_block("instructions")];

    let hybrid = build_openai_request_body_cached(
        "gpt-5.6",
        1024,
        &system,
        &[],
        &[],
        false,
        Some(&placement(None, false)),
    );
    assert!(
        hybrid.get("prompt_cache_options").is_none(),
        "hybrid keeps OpenAI's implicit breakpoints alongside ours"
    );
    assert_eq!(hybrid["prompt_cache_key"], "archon:test");

    let explicit = build_openai_request_body_cached(
        "gpt-5.6",
        1024,
        &system,
        &[],
        &[],
        false,
        Some(&placement(None, true)),
    );
    assert_eq!(explicit["prompt_cache_options"]["mode"], "explicit");
}

// ---------------------------------------------------------------------------
// The cache key
// ---------------------------------------------------------------------------

/// OpenAI routes on a hash of the leading tokens *combined with* the key, so a
/// key that changed between turns would defeat the cache it was meant to
/// address.
#[test]
fn the_cache_key_is_stable_across_turns_and_distinct_across_agents() {
    let agent_a = vec![text_block("you are agent A"), text_block("turn one")];
    let agent_a_later = vec![text_block("you are agent A"), text_block("turn nine")];
    let agent_b = vec![text_block("you are agent B"), text_block("turn one")];

    assert_eq!(
        prompt_cache_key(&agent_a, Some(1)),
        prompt_cache_key(&agent_a_later, Some(1)),
        "the volatile tail must not enter the key"
    );
    assert_ne!(
        prompt_cache_key(&agent_a, Some(1)),
        prompt_cache_key(&agent_b, Some(1)),
        "different prefixes would collide in the cache if they shared a key"
    );
}

// ---------------------------------------------------------------------------
// The size gate
// ---------------------------------------------------------------------------

/// Under the minimum the breakpoint is discarded in silence. Emitting one
/// anyway is not merely useless: paired with `explicit_only` it would turn the
/// implicit breakpoints off and leave the request with no caching at all.
#[test]
fn a_prompt_below_the_minimum_gets_no_placement() {
    let extra = serde_json::json!({
        OPENAI_CACHE_DIRECTIVE_KEY: { "min_tokens": 1024, "explicit_only": true }
    });
    let small = vec![text_block("short")];

    assert!(archon_llm::cache_wire::openai_cache_placement(&extra, &small, &[], &[]).is_none());

    let large = vec![text_block(&"x".repeat(8192))];
    assert!(archon_llm::cache_wire::openai_cache_placement(&extra, &large, &[], &[]).is_some());
}

/// No directive means the request never went through archon's resolution — an
/// SDK caller, or a test. It must not get breakpoints by default.
#[test]
fn no_directive_means_no_breakpoint() {
    let request = LlmRequest::default();

    assert!(
        archon_llm::cache_wire::openai_cache_placement(
            &request.extra,
            &request.system,
            &request.messages,
            &request.tools,
        )
        .is_none()
    );
}
