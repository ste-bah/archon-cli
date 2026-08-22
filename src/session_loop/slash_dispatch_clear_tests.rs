//! Every spelling of `/clear` has to actually clear.
//!
//! `/clear` is a privacy boundary, not a convenience: at the store it empties
//! the log *and* the four relations that keep readable copies of it — the
//! compaction segments, their verbatim bodies, the compaction ledger, and the
//! cached projections. The alias `/cls` reached none of that. It fell past an
//! interception that compared against the literal `/clear`, landed on a
//! handler whose body was `Ok(())`, and returned success without clearing
//! anything or saying so.
//!
//! So these tests refuse to settle for "the handler was called" or "an event
//! was emitted" — that is exactly the evidence the broken version could have
//! produced. Each spelling goes through the real `dispatch_slash_or_skill`
//! with a real `SessionStore`, and afterwards the relations are counted with
//! queries the clear path does not use. What is asserted is that the
//! conversation is gone.
//!
//! The spellings come from `command::clear::spellings()`, which reads
//! `ClearHandler::aliases()` — the same list `RegistryBuilder::build` indexes.
//! A fourth alias declared on the handler enters this test the moment it is
//! declared, and fails here if it does not reach the clear body.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use archon_core::agent::{Agent, AgentConfig, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_llm::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_session::storage::{CompactionLedgerRecord, SessionStore};
use cozo::{DataValue, ScriptMutability};

use super::{SlashDispatchContext, SlashDispatchResult, dispatch_slash_or_skill};

/// The session loop needs an `Agent`, and an `Agent` needs a provider. Clearing
/// never talks to one; a call here means the test wandered into a turn.
struct UnusedProvider;

#[async_trait::async_trait]
impl LlmProvider for UnusedProvider {
    fn name(&self) -> &str {
        "unused-provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        Err(LlmError::Unsupported(
            "the clear-alias fixture must not run a turn".to_string(),
        ))
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Unsupported(
            "the clear-alias fixture must not run a turn".to_string(),
        ))
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        DataFlowClassification::Local
    }
}

/// What the store still holds for a session, read back through queries the
/// clear path never runs.
#[derive(Debug, Default, PartialEq, Eq)]
struct StoredConversation {
    messages: usize,
    compaction_segments: usize,
    compaction_segment_bodies: usize,
    compaction_ledger: usize,
    session_projections: usize,
}

impl StoredConversation {
    fn read(store: &SessionStore, session_id: &str, segment_id: &str) -> Self {
        Self {
            messages: count(
                store,
                "?[message_index] := *messages{session_id, message_index}, session_id = $key",
                session_id,
            ),
            compaction_segments: count(
                store,
                "?[id] := *compaction_segments{id, session_id}, session_id = $key",
                session_id,
            ),
            // Bodies are keyed by segment id, so they are counted by the id
            // captured before the clear — the row cannot be found by joining
            // through a segment that is supposed to be gone.
            compaction_segment_bodies: count(
                store,
                "?[body] := *compaction_segment_bodies{id, body}, id = $key",
                segment_id,
            ),
            compaction_ledger: count(
                store,
                "?[id] := *compaction_ledger{id, session_id}, session_id = $key",
                session_id,
            ),
            session_projections: count(
                store,
                "?[projection_key] := *session_projections{session_id, projection_key}, \
                 session_id = $key",
                session_id,
            ),
        }
    }
}

fn count(store: &SessionStore, script: &str, key: &str) -> usize {
    let mut params = BTreeMap::new();
    params.insert("key".to_string(), DataValue::from(key));
    store
        .db()
        .run_script(script, params, ScriptMutability::Immutable)
        .expect("read the session store back")
        .rows
        .len()
}

fn message(index: usize) -> String {
    serde_json::json!({
        "role": "user",
        "content": format!("the private matter, part {index}"),
    })
    .to_string()
}

/// A session carrying everything a clear has to remove: a log, a closed and
/// summarised compaction segment (which brings its verbatim body with it), a
/// ledger record, and a cached projection. Returns the segment's id, which is
/// how the body is found once the segment itself is gone.
fn seed_conversation(store: &SessionStore, session_id: &str) -> String {
    for index in 0..8 {
        store
            .save_message(session_id, index as u64, &message(index))
            .expect("save message");
    }

    let log = store.load_messages(session_id).expect("load messages");
    let segment = store
        .close_compaction_segment(session_id, 0, 3, &log[0..=3])
        .expect("close compaction segment");
    let claim = store
        .claim_compaction_segment_summary(&segment.id, "test-model", "test-attribution")
        .expect("claim summary")
        .expect("a freshly closed segment is claimable");
    assert!(
        store
            .complete_compaction_segment_summary(
                &segment.id,
                &claim,
                "the private matter, summarised",
                1,
                1,
                0.0,
            )
            .expect("complete summary"),
        "the summary must land on the segment"
    );

    store
        .put_compaction_ledger_record(&CompactionLedgerRecord {
            id: format!("ledger:{session_id}:0"),
            session_id: session_id.to_string(),
            kind: "fact".to_string(),
            payload: "a fact drawn from the private matter".to_string(),
            source_start_index: 0,
            source_end_index: 3,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .expect("put ledger record");

    // Fold a projection so the cache relation has a row to survive the clear.
    store
        .project(
            session_id,
            &archon_session::projection_stats::SessionStatsProjection,
        )
        .expect("project session stats");

    segment.id
}

/// Run `input` through the real slash dispatch against a real store, and
/// report what the store still holds afterwards.
async fn dispatch_and_read_back(input: &str) -> (SlashDispatchResult, StoredConversation) {
    let mut fixture = crate::command::context::slash_ctx_test_fixture::build_test_slash_context(
        "clear-alias-fixture",
        "default",
        None,
        None,
    );
    let session_store = Arc::clone(&fixture.ctx.session_store);
    let session = session_store
        .create_session("/tmp/clear-alias", Some("main"), "test-model")
        .expect("create session");
    let session_id = session.id.clone();
    fixture.ctx.session_id = session_id.clone();

    let segment_id = seed_conversation(&session_store, &session_id);
    let before = StoredConversation::read(&session_store, &session_id, &segment_id);
    assert_eq!(
        before,
        StoredConversation {
            messages: 8,
            compaction_segments: 1,
            compaction_segment_bodies: 1,
            compaction_ledger: 1,
            session_projections: 1,
        },
        "the fixture must seed every relation the clear is supposed to purge, \
         or an empty relation afterwards proves nothing"
    );

    let (agent_event_tx, _agent_event_rx) = tokio::sync::mpsc::channel::<TimestampedEvent>(64);
    let mut agent = Agent::new(
        Arc::new(UnusedProvider),
        ToolRegistry::new(),
        AgentConfig {
            session_id: session_id.clone(),
            model: "test-model".to_string(),
            working_dir: fixture.ctx.working_dir.clone(),
            ..AgentConfig::default()
        },
        agent_event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::empty())),
    );
    agent.set_session_store(Arc::clone(&session_store));
    let agent = Arc::new(tokio::sync::Mutex::new(agent));

    let adapter = Arc::new(crate::agent_handle::AgentHandle::new(
        Arc::clone(&agent),
        session_id.clone(),
        None,
        None,
    ));
    let agent_dispatcher = Arc::clone(&fixture.ctx.agent_dispatcher);
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let mut fast_mode = archon_llm::fast_mode::FastModeState::new_with(false);
    let mut effort_state = archon_llm::effort::EffortState::new();
    let mut post_turn_queue = VecDeque::new();

    let result = dispatch_slash_or_skill(
        input,
        SlashDispatchContext {
            agent: &agent,
            api_url: &None,
            input_tui_tx: &tui_tx,
            session_store: &session_store,
            session_id: &session_id,
            persist_personality: false,
            personality_history_limit: 0,
            session_start_confidence: 0.0,
            session_start_instant: Instant::now(),
            fast_mode: &mut fast_mode,
            effort_state: &mut effort_state,
            cmd_ctx: &mut fixture.ctx,
            dispatcher: &agent_dispatcher,
            adapter: &adapter,
            post_turn_queue: &mut post_turn_queue,
        },
    )
    .await;

    let after = StoredConversation::read(&session_store, &session_id, &segment_id);
    (result, after)
}

/// The claim the fix rests on: typing any spelling of clear leaves nothing of
/// the conversation in the store.
///
/// Table-driven over `command::clear::spellings()`. With the interception back
/// to matching the literal `/clear`, the `/cls` row fails on `messages: 8` —
/// the conversation still fully readable after the user was told nothing at
/// all.
#[tokio::test]
async fn every_spelling_of_clear_empties_the_conversation_from_the_store() {
    for spelling in crate::command::clear::spellings() {
        let input = format!("/{spelling}");
        let (result, after) = dispatch_and_read_back(&input).await;

        assert!(
            result.is_handled(),
            "{input} must be handled by the session loop, got {result:?}"
        );
        assert_eq!(
            after,
            StoredConversation::default(),
            "{input} left the conversation in the store: the user was shown no \
             error, so they believe it is gone"
        );
    }
}

/// The narrower half, stated in the store's own terms: the four relations that
/// used to outlive a clear must be empty for every spelling, not just for the
/// primary one.
#[tokio::test]
async fn every_spelling_of_clear_purges_what_the_store_derived_from_the_log() {
    for spelling in crate::command::clear::spellings() {
        let input = format!("/{spelling}");
        let (_, after) = dispatch_and_read_back(&input).await;

        assert_eq!(
            after.compaction_segments, 0,
            "{input} left a compaction segment; addressed by log index, it \
             re-attaches to the next conversation in this session"
        );
        assert_eq!(
            after.compaction_segment_bodies, 0,
            "{input} left the segment's verbatim body in the store"
        );
        assert_eq!(
            after.compaction_ledger, 0,
            "{input} left facts drawn from the cleared conversation"
        );
        assert_eq!(
            after.session_projections, 0,
            "{input} left a cached projection of the cleared conversation"
        );
    }
}

/// A stray argument must not turn the privacy operation back into silence:
/// `/clear now` clears, it does not quietly do nothing.
#[tokio::test]
async fn clear_with_a_stray_argument_still_clears() {
    let (result, after) = dispatch_and_read_back("/cls now").await;
    assert!(result.is_handled());
    assert_eq!(after, StoredConversation::default());
}

/// The guard on the other side: the interception must not swallow a command
/// that merely starts with the same letters.
#[tokio::test]
async fn a_different_command_does_not_reach_the_clear_body() {
    let (_, after) = dispatch_and_read_back("/clearly-not-a-command").await;
    assert_eq!(
        after.messages, 8,
        "an unrelated command cleared the conversation"
    );
}
