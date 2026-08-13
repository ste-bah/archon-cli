//! Live Bedrock verification: does a checkpoint actually get written and read?
//!
//! Ignored by default — it costs money and needs AWS credentials. Run it
//! deliberately:
//!
//! ```bash
//! cargo test -p archon-llm --test bedrock_live_cache -- --ignored --nocapture
//! ```
//!
//! Set `ARCHON_BEDROCK_TEST_MODELS` to a comma-separated list of inference
//! profile ids to override the defaults, and `AWS_REGION` for the region.
//!
//! This exists because none of the unit tests can catch the failure that
//! matters. A checkpoint below the model's minimum is discarded **in silence**:
//! the request succeeds, the body looks right, every assertion about the JSON
//! passes, and the prompt is billed in full on every turn. The only thing that
//! distinguishes a working cache from a broken one is what Bedrock reports back,
//! so that is what this asserts on.
//!
//! It exercises the shipped path end to end — `build_converse_body_cached` for
//! the `cachePoint` elements, SigV4 signing, and `decode_eventstream_frames`
//! for the binary response — rather than a hand-rolled request.

use archon_llm::cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY;
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::BedrockProvider;
use archon_llm::streaming::StreamEvent;

/// Models proven invocable on the test account in eu-west-2.
///
/// The spellings are load-bearing: several models are reachable only through a
/// dated, versioned id, and the undecorated form returns `ValidationException`
/// rather than anything that hints at the right answer.
const DEFAULT_MODELS: &[&str] = &[
    "eu.anthropic.claude-sonnet-4-6",
    "eu.anthropic.claude-haiku-4-5-20251001-v1:0",
    "eu.anthropic.claude-opus-4-6-v1",
    "eu.anthropic.claude-sonnet-4-5-20250929-v1:0",
    "eu.anthropic.claude-opus-4-5-20251101-v1:0",
];

/// Comfortably above the highest minimum in the table (4,096) so that one
/// prompt serves every model, and stable byte-for-byte across both calls —
/// which is the entire premise of a prefix cache.
fn stable_system_prompt() -> String {
    let paragraph = "You are a careful assistant. Answer precisely and briefly. \
                     Prefer facts over speculation, and say so when you are unsure. ";
    paragraph.repeat(400)
}

fn request(model: &str, system: &str, user: &str, min_tokens: usize) -> LlmRequest {
    LlmRequest {
        model: model.to_string(),
        max_tokens: 32,
        system: vec![serde_json::json!({ "type": "text", "text": system })],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{ "type": "text", "text": user }]
        })],
        extra: serde_json::json!({
            BEDROCK_CACHE_DIRECTIVE_KEY: {
                "max": 4,
                "min_tokens": min_tokens,
                "ttl_1h": false,
                "conversation": true,
            }
        }),
        ..LlmRequest::default()
    }
}

/// Drain a stream and return `(cache_write, cache_read)` from the usage event.
async fn usage_from(provider: &BedrockProvider, request: LlmRequest) -> (u64, u64) {
    let mut rx = provider.stream(request).await.expect("stream opens");
    let mut write = 0;
    let mut read = 0;
    let mut saw_usage = false;

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::MessageStart { usage, .. } => {
                write = usage.cache_creation_input_tokens;
                read = usage.cache_read_input_tokens;
                saw_usage = true;
            }
            StreamEvent::MessageDelta {
                usage: Some(usage), ..
            } => {
                // Bedrock reports the cache counters on the metadata event at
                // the end of the stream; take the later figure where both
                // appear.
                if usage.cache_creation_input_tokens > 0 || usage.cache_read_input_tokens > 0 {
                    write = usage.cache_creation_input_tokens;
                    read = usage.cache_read_input_tokens;
                }
                saw_usage = true;
            }
            StreamEvent::Error {
                error_type,
                message,
            } => panic!("stream error {error_type}: {message}"),
            _ => {}
        }
    }

    assert!(
        saw_usage,
        "no usage event — the eventstream decode is producing nothing, which is \
         exactly how a turn completes in seconds with no content and no error"
    );
    (write, read)
}

#[tokio::test]
#[ignore = "live Bedrock: costs money, needs AWS credentials"]
async fn a_checkpoint_is_written_and_then_read_on_every_model() {
    let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "eu-west-2".to_string());
    let models: Vec<String> = std::env::var("ARCHON_BEDROCK_TEST_MODELS")
        .map(|raw| raw.split(',').map(|m| m.trim().to_string()).collect())
        .unwrap_or_else(|_| DEFAULT_MODELS.iter().map(|m| (*m).to_string()).collect());

    let system = stable_system_prompt();
    let mut failures = Vec::new();

    for model in &models {
        let provider = BedrockProvider::new(region.clone(), model.clone());

        // The minimum Archon itself would apply for this model on Bedrock —
        // resolved through the provider, so a wrong entry in the table shows up
        // here as a discarded checkpoint rather than passing quietly.
        let min_tokens = provider.cache_strategy(model).min_tokens();

        let (write_1, read_1) =
            usage_from(&provider, request(model, &system, "Say hi.", min_tokens)).await;

        // A *different* user message on the same prefix. If the checkpoint
        // works, the system prompt is a read and only the new message is fresh
        // input — which is the case that actually saves money, not a byte-for-
        // byte repeat.
        let (write_2, read_2) =
            usage_from(&provider, request(model, &system, "Say hello.", min_tokens)).await;

        println!(
            "{model}: min_tokens={min_tokens} \
             turn1 write={write_1} read={read_1}  turn2 write={write_2} read={read_2}"
        );

        // Either counter moving proves the checkpoint reached the wire and
        // cleared the minimum. A read on the *first* turn is not a failure —
        // it means an earlier run left the prefix warm, since the retention is
        // five minutes and this test is cheap enough to run twice.
        if write_1 == 0 && read_1 == 0 {
            failures.push(format!(
                "{model}: neither counter moved — the checkpoint fell under the \
                 {min_tokens}-token minimum and was discarded in silence, or no \
                 cachePoint reached the wire"
            ));
        }
        if read_2 == 0 {
            failures.push(format!(
                "{model}: turn 2 read nothing back. A write that is never read \
                 bills at 1.25x input, so this is worse than not caching"
            ));
        }
        if write_1 > 0 && read_2 > 0 && read_2 != write_1 {
            println!(
                "  note: {model} read {read_2} against {write_1} written — a \
                 partial prefix hit, not a failure"
            );
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
