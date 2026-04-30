//! Personality snapshot save helper.
//!
//! Extracted from byte-identical save blocks in `session_loop/mod.rs`
//! at the `/exit | /quit | /q` and `/clear` slash command handlers
//! (TASK #242 CONSCIOUSNESS-PERSIST-1).
//!
//! The helper wraps the `persist_personality` gate, the inner-voice
//! Some/None check, the `RulesEngine::export_scores` + `to_session_stats`
//! + `on_compaction` snapshot construction, and both `save_snapshot` and
//! `prune_snapshots` calls. Callers pass the dependencies directly
//! (memory, session_id, gating flags) so the helper is unit-testable
//! without a full `Agent` or `SlashCommandContext`.

use std::sync::Arc;

use archon_consciousness::inner_voice::InnerVoice;
use archon_memory::access::MemoryTrait;

/// Save a personality snapshot if the feature is enabled and an inner
/// voice is present. No-op otherwise.
///
/// Two short-circuits:
/// - `persist_personality == false` → return immediately.
/// - `iv_arc == None` → return immediately (no inner voice configured).
///
/// Errors from `save_snapshot` / `prune_snapshots` are logged at WARN and
/// swallowed — losing a snapshot is not worth aborting session shutdown.
pub(crate) async fn save_personality_snapshot_if_enabled(
    iv_arc: Option<Arc<tokio::sync::Mutex<InnerVoice>>>,
    memory: &dyn MemoryTrait,
    session_id: &str,
    persist_personality: bool,
    personality_history_limit: u32,
    session_start_confidence: f32,
    session_start_instant: std::time::Instant,
) {
    if !persist_personality {
        return;
    }
    let Some(iv_arc) = iv_arc else {
        return;
    };

    let iv = iv_arc.lock().await;
    let stats = iv.to_session_stats(
        session_start_confidence,
        session_start_instant.elapsed().as_secs(),
    );
    let snapshot_iv = iv.on_compaction();
    drop(iv);

    let engine = archon_consciousness::rules::RulesEngine::new(memory);
    let rule_scores = engine.export_scores().unwrap_or_default();

    let snap = archon_consciousness::persistence::PersonalitySnapshot {
        session_id: session_id.to_string(),
        timestamp: chrono::Utc::now(),
        inner_voice: snapshot_iv,
        rule_scores,
        stats,
    };

    if let Err(e) = archon_consciousness::persistence::save_snapshot(memory, &snap) {
        tracing::warn!("personality: failed to save snapshot: {e}");
    }
    if let Err(e) =
        archon_consciousness::persistence::prune_snapshots(memory, personality_history_limit)
    {
        tracing::warn!("personality: failed to prune snapshots: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::MemoryGraph;

    #[tokio::test]
    async fn no_op_when_persist_disabled() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let iv = Arc::new(tokio::sync::Mutex::new(InnerVoice::new()));

        save_personality_snapshot_if_enabled(
            Some(iv),
            &graph,
            "sess-disabled",
            false, // persist_personality = false → short-circuit
            10,
            0.7,
            std::time::Instant::now(),
        )
        .await;

        let result = archon_consciousness::persistence::load_latest_snapshot(&graph).expect("load");
        assert!(
            result.is_none(),
            "no snapshot should exist when persist_personality is false"
        );
    }

    #[tokio::test]
    async fn no_op_when_inner_voice_none() {
        let graph = MemoryGraph::in_memory().expect("graph");

        save_personality_snapshot_if_enabled(
            None, // no inner voice → short-circuit
            &graph,
            "sess-no-iv",
            true,
            10,
            0.7,
            std::time::Instant::now(),
        )
        .await;

        let result = archon_consciousness::persistence::load_latest_snapshot(&graph).expect("load");
        assert!(
            result.is_none(),
            "no snapshot should exist when iv_arc is None"
        );
    }

    #[tokio::test]
    async fn saves_snapshot_when_enabled() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let iv = Arc::new(tokio::sync::Mutex::new(InnerVoice::new()));

        save_personality_snapshot_if_enabled(
            Some(iv),
            &graph,
            "sess-saved",
            true,
            10,
            0.5,
            std::time::Instant::now(),
        )
        .await;

        let result = archon_consciousness::persistence::load_latest_snapshot(&graph)
            .expect("load")
            .expect("snapshot present after save");
        assert_eq!(result.session_id, "sess-saved");
    }

    #[tokio::test]
    async fn save_then_prune_keeps_only_latest() {
        let graph = MemoryGraph::in_memory().expect("graph");

        for i in 0..3 {
            let iv = Arc::new(tokio::sync::Mutex::new(InnerVoice::new()));
            save_personality_snapshot_if_enabled(
                Some(iv),
                &graph,
                &format!("sess-{i}"),
                true,
                1, // history_limit = 1, expect prune to keep just the newest
                0.5,
                std::time::Instant::now(),
            )
            .await;
        }

        let latest = archon_consciousness::persistence::load_latest_snapshot(&graph)
            .expect("load")
            .expect("at least one snapshot");
        assert_eq!(
            latest.session_id, "sess-2",
            "newest snapshot should be sess-2"
        );
    }
}
