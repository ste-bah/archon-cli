//! The detached pass: what is allowed to start it, what it is allowed to
//! change, and what it says afterwards.

use super::*;

/// Run the background pass to completion against a given store, as production
/// would run it detached.
///
/// Returns `None` when the trigger declined to fire, which is a different
/// outcome from firing and merging nothing — the whole point of the config gate
/// is that no provider is reached at all.
async fn background(
    config: &archon_memory::garden::GardenConfig,
    client: Arc<dyn archon_pipeline::runner::LlmClient>,
    store: Arc<dyn MemoryTrait>,
    pairs: Vec<ReviewPair>,
) -> Option<usize> {
    let handle = spawn_review_band_adjudication(
        config,
        client,
        store,
        pairs,
        "test-model".to_string(),
        None,
    )?;
    Some(handle.await.expect("background adjudication task panicked"))
}

fn band(count: usize) -> Vec<ReviewPair> {
    (0..count)
        .map(|i| pair(&format!("a{i}"), &format!("b{i}")))
        .collect()
}

/// The default configuration must not reach a provider at all.
///
/// Not "merges nothing" — makes no call, and starts no task. Automatic
/// consolidation runs before the user has typed anything, and the whole reason
/// this is opt-in is that the call itself is the cost.
#[tokio::test]
async fn the_automatic_path_is_silent_by_default() {
    let client = Arc::new(RecordingClient::default());
    let config = archon_memory::garden::GardenConfig::default();

    let outcome = background(&config, client.clone(), empty_store(), band(50)).await;

    assert_eq!(outcome, None, "the default must not start a pass at all");
    assert_eq!(
        client.calls(),
        0,
        "the default must not spend a round-trip at session start"
    );
}

/// Enabled but under the threshold: still no call.
///
/// The threshold is the half of this feature that keeps "adjudicate
/// automatically" from meaning "one LLM call every launch".
#[tokio::test]
async fn a_band_under_the_threshold_makes_no_call() {
    let client = Arc::new(RecordingClient::default());

    let outcome = background(&adjudicating(10), client.clone(), empty_store(), band(9)).await;

    assert_eq!(outcome, None);
    assert_eq!(
        client.calls(),
        0,
        "nine pairs must not trigger a ten-pair threshold"
    );
}

/// Enabled and exactly at the threshold: one call, judging every pending pair.
#[tokio::test]
async fn a_band_at_the_threshold_is_judged_in_one_call() {
    let client = Arc::new(RecordingClient::default());

    background(&adjudicating(10), client.clone(), empty_store(), band(10))
        .await
        .expect("the threshold was met, so a pass must have started");

    assert_eq!(client.calls(), 1);
    assert_eq!(
        numbered_pairs(&client.last_prompt()),
        10,
        "a run that fires at the threshold must clear the band it fired on"
    );
}

/// Blocks in `send_message` until a permit is issued, so a test can observe the
/// state of the world while the provider is still thinking.
///
/// A semaphore rather than a `Notify`, because the release can be issued before
/// the task has reached the wait: a permit is remembered, a notification sent to
/// nobody is not.
struct GatedClient {
    release: Arc<tokio::sync::Semaphore>,
    reached_provider: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl archon_pipeline::runner::LlmClient for GatedClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        self.reached_provider
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _permit = self.release.acquire().await.expect("gate closed");
        Ok(archon_pipeline::runner::LlmResponse {
            content: "1: SAME".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

/// The session must not wait for the verdict.
///
/// This is the whole reason the pass is detached rather than awaited: it runs
/// during session bootstrap, before the TUI exists and before the user can type,
/// and it is the only thing on that path that talks to a model. The assertion is
/// that the CALLER returns while the provider is still blocked — the gate holds
/// no permits, so the pass provably cannot have completed. A timing bound alone
/// would pass just as well against an awaited call to a fast double.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starting_the_background_pass_does_not_wait_for_the_provider() {
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let reached_provider = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let client = Arc::new(GatedClient {
        release: Arc::clone(&release),
        reached_provider: Arc::clone(&reached_provider),
    });

    let began = std::time::Instant::now();
    let handle = spawn_review_band_adjudication(
        &adjudicating(1),
        client,
        empty_store(),
        band(1),
        "test-model".to_string(),
        None,
    )
    .expect("the threshold was met, so a pass must have started");
    let elapsed = began.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "session bootstrap waited {elapsed:?} on an adjudication that has not answered"
    );
    assert!(
        !handle.is_finished(),
        "the pass must still be in flight; the provider has not been released"
    );

    release.add_permits(1);
    handle.await.expect("background adjudication task panicked");
    assert!(
        reached_provider.load(std::sync::atomic::Ordering::SeqCst),
        "the pass must have reached the provider once detached"
    );
}

/// Seed two live memories and hand back a review pair naming them.
fn stored_pair(store: &dyn MemoryTrait, a: &str, b: &str) -> ReviewPair {
    let store_one = |content: &str| {
        store
            .store_memory(
                content,
                content,
                archon_memory::types::MemoryType::Fact,
                0.5,
                &[],
                "test",
                "",
            )
            .expect("store memory")
    };
    let a_id = store_one(a);
    let b_id = store_one(b);
    ReviewPair {
        a_id,
        b_id,
        a_content: a.to_string(),
        b_content: b.to_string(),
    }
}

fn is_live(store: &dyn MemoryTrait, id: &str) -> bool {
    let memory = store.inspect_memory(id).expect("memory still exists");
    !archon_memory::types::is_superseded(&memory.tags)
}

/// A verdict arriving after startup still merges, through the adjudicated path.
///
/// Detaching the pass must not have detached it from its effect. The merge is a
/// supersession, not a delete: both rows survive, one marked.
#[tokio::test]
async fn a_verdict_arriving_after_startup_is_applied_to_the_store() {
    let store = empty_store();
    let judged = stored_pair(
        store.as_ref(),
        "Deploy region is eu-west-2",
        "All deploys target eu-west-2",
    );
    let (a_id, b_id) = (judged.a_id.clone(), judged.b_id.clone());

    let merged = background(
        &adjudicating(1),
        Arc::new(RecordingClient::default()),
        Arc::clone(&store),
        vec![judged],
    )
    .await
    .expect("the threshold was met, so a pass must have started");

    assert_eq!(merged, 1, "a SAME verdict must fold the pair");
    assert!(
        is_live(store.as_ref(), &a_id) != is_live(store.as_ref(), &b_id),
        "exactly one of the pair must survive; the other is marked superseded"
    );
}

/// Answers DIFFERENT to everything.
#[derive(Default)]
struct DecliningClient;

#[async_trait::async_trait]
impl archon_pipeline::runner::LlmClient for DecliningClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        Ok(archon_pipeline::runner::LlmResponse {
            content: (1..=500)
                .map(|i| format!("{i}: DIFFERENT"))
                .collect::<Vec<_>>()
                .join("\n"),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

/// Fails every call.
#[derive(Default)]
struct FailingClient;

#[async_trait::async_trait]
impl archon_pipeline::runner::LlmClient for FailingClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        Err(anyhow::anyhow!("provider unreachable"))
    }
}

/// Nothing but an explicit SAME may touch a stored memory.
///
/// The reason this is asserted against a real store rather than against the
/// parser: an earlier version marked review-band pairs with a `RelatedTo` edge,
/// a LATER phase read that mark as a merge instruction, and 13 memories were
/// destroyed by pairs the band had deliberately declined to merge. So the
/// property that matters is not "the verdict was DIFFERENT" but "the store is
/// bit-for-bit as unjudged as it was before" — no supersession, no new edge, no
/// tag. A provider that errors outright must leave the same nothing behind.
#[tokio::test]
async fn no_verdict_and_a_declined_verdict_both_leave_the_store_untouched() {
    for (label, client) in [
        (
            "DIFFERENT",
            Arc::new(DecliningClient) as Arc<dyn archon_pipeline::runner::LlmClient>,
        ),
        ("a failed call", Arc::new(FailingClient)),
    ] {
        let store = empty_store();
        let judged = stored_pair(
            store.as_ref(),
            "Deploy region is eu-west-2",
            "Never deploy to us-east-1",
        );
        let (a_id, b_id) = (judged.a_id.clone(), judged.b_id.clone());

        let merged = background(&adjudicating(1), client, Arc::clone(&store), vec![judged])
            .await
            .expect("the threshold was met, so a pass must have started");

        assert_eq!(merged, 0, "{label} must merge nothing");
        assert!(is_live(store.as_ref(), &a_id), "{label} must spare A");
        assert!(is_live(store.as_ref(), &b_id), "{label} must spare B");
        assert!(
            store
                .get_related_memories(&a_id, 1)
                .expect("related lookup")
                .is_empty(),
            "{label} must leave no edge behind for a later phase to read as a merge"
        );
    }
}

/// A merge the user never asked for is reported where the user will see it.
///
/// The startup panel has already been drawn by the time a verdict arrives, and
/// it said those pairs were outstanding. Without this the correction exists only
/// in the log, and a process that reshapes memory with no visible record is one
/// whose mistakes are indistinguishable from it working.
#[tokio::test]
async fn merges_made_in_the_background_are_reported_to_the_session() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let store = empty_store();
    let judged = stored_pair(
        store.as_ref(),
        "Deploy region is eu-west-2",
        "All deploys target eu-west-2",
    );

    let merged = spawn_review_band_adjudication(
        &adjudicating(1),
        Arc::new(RecordingClient::default()),
        Arc::clone(&store),
        vec![judged],
        "test-model".to_string(),
        Some(tx),
    )
    .expect("the threshold was met, so a pass must have started")
    .await
    .expect("background adjudication task panicked");
    assert_eq!(merged, 1);

    let mut text = String::new();
    while let Ok(archon_tui::app::TuiEvent::TextDelta(delta)) = rx.try_recv() {
        text.push_str(&delta);
    }
    assert!(
        text.contains("Memory garden") && text.contains("merged"),
        "the background merge was never surfaced; got {text:?}"
    );
}

/// A pass that changed nothing says nothing.
///
/// Consolidation fires on every session start once the throttle elapses. "The
/// background judged your memories and changed none of them" on every launch is
/// a line people learn to skip, and a notice everyone skips is no more visible
/// than the log line it replaced.
#[tokio::test]
async fn a_background_pass_that_merges_nothing_is_silent() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel();

    spawn_review_band_adjudication(
        &adjudicating(1),
        Arc::new(DecliningClient),
        empty_store(),
        band(3),
        "test-model".to_string(),
        Some(tx),
    )
    .expect("the threshold was met, so a pass must have started")
    .await
    .expect("background adjudication task panicked");

    assert!(
        rx.try_recv().is_err(),
        "a pass that merged nothing must not interrupt the session"
    );
}
