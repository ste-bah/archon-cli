//! Live end-to-end checks against a real vLLM server (#123).
//!
//! These are `#[ignore]`d: they need a running server and must never run in
//! CI. Run them deliberately:
//!
//! ```text
//! ARCHON_LIVE_VLLM_URL=http://192.168.1.27:8888/v1 \
//! ARCHON_LIVE_VLLM_MODEL=deepseek-v4-flash-dspark \
//!   cargo test -p archon-llm --test vllm_live -- --ignored --nocapture
//! ```
//!
//! They exist because the unit tests can only prove archon writes the body it
//! intends to write. Only a real server proves the server agrees — and the
//! empirical work behind this issue found three places where the obvious
//! assumption was wrong: the streamed field is `reasoning` rather than
//! `reasoning_content`, `reasoning_effort` inside `chat_template_kwargs` is
//! inert without `thinking`, and an unknown kwarg returns 200 rather than an
//! error.

use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::LocalProvider;
use archon_llm::reasoning::{ReasoningConfig, ReasoningMode};
use archon_llm::streaming::StreamEvent;

fn live_config() -> Option<(String, String)> {
    let url = std::env::var("ARCHON_LIVE_VLLM_URL").ok()?;
    let model = std::env::var("ARCHON_LIVE_VLLM_MODEL")
        .unwrap_or_else(|_| "deepseek-v4-flash-dspark".to_string());
    Some((url, model))
}

/// Stream one short request and return `(thinking_chars, text_chars)`.
async fn run(reasoning: ReasoningConfig, effort: Option<&str>) -> (usize, usize) {
    let (url, model) = live_config().expect("ARCHON_LIVE_VLLM_URL must be set for live tests");
    let provider = LocalProvider::new(url, model.clone(), 120, false).with_reasoning(reasoning);

    let mut rx = provider
        .stream(LlmRequest {
            model,
            max_tokens: 256,
            messages: vec![serde_json::json!({
                "role": "user",
                "content": "What is 17 * 23? Answer with the number only."
            })],
            effort: effort.map(str::to_string),
            ..LlmRequest::default()
        })
        .await
        .expect("live vLLM stream should start");

    let (mut thinking, mut text) = (0usize, 0usize);
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ThinkingDelta { thinking: t, .. } => thinking += t.len(),
            StreamEvent::TextDelta { text: t, .. } => text += t.len(),
            _ => {}
        }
    }
    (thinking, text)
}

fn top_level() -> ReasoningConfig {
    ReasoningConfig {
        mode: ReasoningMode::TopLevel,
        ..ReasoningConfig::default()
    }
}

/// The acceptance criterion: with reasoning configured, `/effort` produces
/// actual reasoning output that archon can see. Fails on the pre-#123 code in
/// two independent ways — nothing is sent, and nothing is parsed.
#[tokio::test]
#[ignore = "requires a live vLLM server; see module docs"]
async fn effort_produces_reasoning_that_archon_can_read() {
    let (thinking, text) = run(top_level(), Some("max")).await;
    assert!(
        thinking > 0,
        "expected reasoning deltas at effort=max; got none (is --reasoning-parser enabled?)"
    );
    assert!(text > 0, "expected an answer alongside the reasoning");
    println!("effort=max -> {thinking} reasoning chars, {text} answer chars");
}

/// The default must not change behaviour for backends with no reasoning
/// switch: nothing is sent, so the template's own default applies.
#[tokio::test]
#[ignore = "requires a live vLLM server; see module docs"]
async fn default_off_sends_no_reasoning_controls() {
    let (thinking, text) = run(ReasoningConfig::default(), Some("max")).await;
    assert!(text > 0, "the model should still answer with reasoning off");
    println!("mode=off -> {thinking} reasoning chars, {text} answer chars");
}

/// `chat_template_kwargs` mode needs `thinking` alongside the level — the
/// level alone is inert on this template. Documents the trap in executable
/// form.
#[tokio::test]
#[ignore = "requires a live vLLM server; see module docs"]
async fn chat_template_kwargs_needs_thinking_to_do_anything() {
    let mut without = ReasoningConfig {
        mode: ReasoningMode::ChatTemplateKwargs,
        ..ReasoningConfig::default()
    };
    without.kwargs.clear();
    let (thinking_without, _) = run(without, Some("max")).await;

    let mut with = ReasoningConfig {
        mode: ReasoningMode::ChatTemplateKwargs,
        ..ReasoningConfig::default()
    };
    with.kwargs
        .insert("thinking".into(), serde_json::json!(true));
    let (thinking_with, _) = run(with, Some("max")).await;

    println!("kwargs without thinking -> {thinking_without}; with thinking -> {thinking_with}");
    assert!(
        thinking_with > thinking_without,
        "`thinking` must be what actually enables reasoning in kwargs mode"
    );
}
