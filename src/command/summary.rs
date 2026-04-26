//! TASK-#209 SLASH-SUMMARY /summary slash-command handler.
//!
//! `/summary` emits a compact one-glance session summary as a single
//! `TuiEvent::TextDelta`. Distinct from its three sibling commands:
//!
//!   - `/usage` — 4-line aligned table (input/output/turns/cost).
//!   - `/extra-usage` (#215) — 6-section grouped detailed report
//!     (SESSION / TOKENS / COSTS / CACHE / EFFICIENCY / NOTES).
//!   - `/cost` — 6-line cost breakdown with warn/hard thresholds.
//!   - `/summary` (this command) — ONE-LINE headline + a one-line
//!     cache summary. Optimised for "what's the state of the
//!     session right now" at a glance.
//!
//! # Architecture
//!
//! Reuses the existing `usage_snapshot` field on `CommandContext`
//! (originally populated for `/usage` in TASK-AGS-POST-6-BODIES-B16,
//! widened in TASK-#215 SLASH-EXTRA-USAGE for `/extra-usage`). This
//! commit widens the snapshot-population arm in `src/command/
//! context.rs` once more — `Some("usage") | Some("extra-usage") |
//! Some("summary") =>` — so the same async builder fires for any of
//! the three readers without standing up a parallel snapshot.
//!
//! # Why TextDelta, not overlay
//!
//! Mission text: "may or may not need overlay (decide based on how
//! other non-overlay text-delta commands display output)". Every
//! peer command in the session-stats family (`/usage`, `/cost`,
//! `/status`, `/extra-usage`) emits `TuiEvent::TextDelta`. `/summary`
//! follows the same precedent — there's no scrollable list to
//! navigate, no per-row selection, no follow-up actions. A single
//! TextDelta is the right primitive.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/summary` handler.
pub(crate) struct SummaryHandler;

impl CommandHandler for SummaryHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let snap = ctx.usage_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "SummaryHandler invoked without usage_snapshot populated \
                 — build_command_context bug (summary arm missing?)"
            )
        })?;

        let session_label = ctx
            .session_id
            .as_deref()
            .map(short_session_id)
            .unwrap_or_else(|| "(none)".to_string());

        let total_tokens = snap.input_tokens.saturating_add(snap.output_tokens);

        let body = render_summary(
            &session_label,
            snap.turn_count,
            total_tokens,
            snap.total_cost,
            &snap.cache_stats_line,
        );
        ctx.emit(TuiEvent::TextDelta(body));
        Ok(())
    }

    fn description(&self) -> &str {
        "Show a one-glance session summary (turns / tokens / cost / cache)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Render the summary body. Pulled out so unit tests exercise the
/// formatting without needing a live `CommandContext` snapshot.
fn render_summary(
    session_label: &str,
    turns: u64,
    total_tokens: u64,
    total_cost: f64,
    cache_stats_line: &str,
) -> String {
    // `cache_stats_line` is three newline-joined lines from the
    // session_stats CacheStats::format_for_cost() output:
    //   "Cache hit rate: X% (Y reads / Z total)"
    //   "Cache creation: W tokens"
    //   "Estimated savings: V token-equivalents"
    // Compact summary takes ONLY the first line so /summary stays
    // single-glance.
    let cache_first_line = cache_stats_line
        .lines()
        .next()
        .unwrap_or("Cache hit rate: n/a");

    format!(
        "\nSession summary\n\
         ───────────────\n\
         ID: {sid}  \u{00b7}  Turns: {turns}  \u{00b7}  Tokens: {tokens}  \u{00b7}  \
         Cost: ${cost:.4}\n\
         {cache_first_line}\n\
         \n\
         Tip: /usage for the flat table, /extra-usage for grouped sections.\n",
        sid = session_label,
        turns = turns,
        tokens = total_tokens,
        cost = total_cost,
        cache_first_line = cache_first_line,
    )
}

/// Shorten a UUID-like session id to its first 8 characters for the
/// at-a-glance summary line. Returns the input unchanged if it's
/// already 8 chars or shorter.
fn short_session_id(id: &str) -> String {
    let max = 8;
    if id.chars().count() <= max {
        return id.to_string();
    }
    id.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn execute_without_snapshot_returns_err() {
        let handler = SummaryHandler;
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("usage_snapshot"),
            "error must reference usage_snapshot; got: {}",
            msg
        );
    }

    #[test]
    fn execute_emits_headline_with_id_turns_tokens_cost() {
        let handler = SummaryHandler;
        let snap = fixture_usage_snapshot(); // 1M in, 500K out, 3 turns, $10.50
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_usage_snapshot(snap)
            .with_session_id("abcd1234efgh5678".into())
            .build();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        let body = match events.into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Session summary"));
        // ID truncated to first 8 chars.
        assert!(body.contains("ID: abcd1234"));
        // Turns + tokens + cost on the headline.
        assert!(body.contains("Turns: 3"));
        // Total tokens = 1_000_000 + 500_000 = 1_500_000.
        assert!(body.contains("Tokens: 1500000"));
        assert!(body.contains("Cost: $10.5000"));
    }

    #[test]
    fn execute_with_no_session_id_renders_none_marker() {
        let handler = SummaryHandler;
        let snap = fixture_usage_snapshot();
        let (mut ctx, mut rx) = CtxBuilder::new().with_usage_snapshot(snap).build();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("ID: (none)"));
    }

    #[test]
    fn execute_emits_first_cache_stats_line_only() {
        // The summary is ONE-glance — only the first line of the
        // three-line cache_stats_line should appear in the rendered
        // output. Subsequent lines (`Cache creation: ...`,
        // `Estimated savings: ...`) belong in /extra-usage.
        let handler = SummaryHandler;
        let snap = fixture_usage_snapshot();
        let (mut ctx, mut rx) = CtxBuilder::new().with_usage_snapshot(snap).build();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Cache hit rate: 0.0%"));
        // Multi-line cache content from the snapshot must NOT bleed
        // into the summary — /summary intentionally drops everything
        // after the first cache line.
        assert!(
            !body.contains("Cache creation"),
            "summary must NOT include the cache_creation line; got:\n{}",
            body
        );
        assert!(
            !body.contains("Estimated savings"),
            "summary must NOT include the estimated_savings line; got:\n{}",
            body
        );
    }

    #[test]
    fn execute_points_at_sibling_commands() {
        let handler = SummaryHandler;
        let snap = fixture_usage_snapshot();
        let (mut ctx, mut rx) = CtxBuilder::new().with_usage_snapshot(snap).build();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        // Tip line cross-references /usage and /extra-usage so users
        // who want more detail know where to look.
        assert!(body.contains("/usage"));
        assert!(body.contains("/extra-usage"));
    }

    #[test]
    fn render_summary_with_zero_turns_renders_zeros() {
        // Edge: brand-new session, no turns yet. Must not panic; must
        // render zeros without dividing by anything.
        let body = render_summary(
            "(none)",
            0,
            0,
            0.0,
            "Cache hit rate: 0.0% (0 reads / 0 total)",
        );
        assert!(body.contains("Turns: 0"));
        assert!(body.contains("Tokens: 0"));
        assert!(body.contains("Cost: $0.0000"));
    }

    #[test]
    fn short_session_id_truncates_long_ids() {
        assert_eq!(
            short_session_id("abcd1234efgh5678ijkl9012mnop3456"),
            "abcd1234"
        );
    }

    #[test]
    fn short_session_id_passes_through_short_input() {
        assert_eq!(short_session_id("short"), "short");
        assert_eq!(short_session_id("exact8ch"), "exact8ch");
    }

    #[test]
    fn short_session_id_unicode_safe() {
        // 4 multi-byte chars + 4 ASCII = 8 chars, should pass through.
        let s: String = "αβγδ1234".to_string();
        assert_eq!(short_session_id(&s), s);
        // 12 multi-byte chars truncate to first 8.
        let s: String = "αβγδεζηθικλμ".to_string();
        let out = short_session_id(&s);
        assert_eq!(out.chars().count(), 8);
    }

    #[test]
    fn description_and_aliases() {
        let h = SummaryHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn summary_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("summary") must return Some;
        // dispatch with a fixture usage_snapshot must emit a single
        // TextDelta containing the Session summary header.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("summary")
            .expect("summary must be registered in default_registry()");

        let snap = fixture_usage_snapshot();
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_usage_snapshot(snap)
            .with_session_id("smoke-test".into())
            .build();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Session summary"));
        assert!(body.contains("Turns:"));
        assert!(body.contains("Tokens:"));
        assert!(body.contains("Cost:"));
    }
}
