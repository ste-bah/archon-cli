//! TASK-#215 SLASH-EXTRA-USAGE — `/extra-usage` detailed-stats handler.
//!
//! `/extra-usage` is a SUPERSET of `/usage`: it groups the same
//! `usage_snapshot` data into 6 labelled sections (SESSION, TOKENS,
//! COSTS, CACHE, EFFICIENCY, NOTES) and adds derived per-turn averages
//! and a cost/1k-tokens efficiency metric. This is the closest the
//! shipped session-stats surface gets to a "detailed usage report"
//! without reaching into infrastructure that is not currently exposed
//! at session scope (see Spec/reality reconciliation below).
//!
//! # Spec/reality reconciliation
//!
//! The ticket spec for #215 said:
//!
//!   "Reads from `archon-core::tasks::metrics::TaskMetrics` + LLM
//!    provider stats. Formats aligned table as `TuiEvent::SystemMessage`."
//!
//! Reality on the `slash-commands-parity` branch (HEAD = ad7123d):
//!
//!   1. `archon-core::tasks::metrics::TaskMetrics` does NOT exist. The
//!      metrics facility in that module is `MetricsRegistry` with 4
//!      atomic counters (started/finished/failed/cancelled) and a
//!      `set_queue_depth` map. A new `MetricsRegistry::new()` is
//!      constructed in EVERY task-subcommand entrypoint
//!      (`src/command/task.rs:65, 85, 105, 126, 146, 162, 178`) — there
//!      is no session-shared instance, so the counters reset to zero
//!      between dispatches and would always read 0/0/0/0 if surfaced
//!      via a snapshot. Plumbing a session-shared `Arc<MetricsRegistry>`
//!      through `SlashCommandContext` + `CommandContext` is
//!      cross-cutting subsystem work that exceeds the wrapper-scope
//!      ceiling for this ticket.
//!   2. LLM "provider stats" are not aggregated at the
//!      `archon-llm::ProviderRegistry` level; per-call `Usage`
//!      (input/output/cache tokens) is returned on each `LlmResponse`
//!      and accumulated into `SessionStats` — which the existing
//!      `usage_snapshot` already exposes verbatim.
//!   3. `TuiEvent::SystemMessage` does NOT exist (see the enum at
//!      `crates/archon-tui/src/app.rs:38-133`). All shipped slash
//!      handlers use `TuiEvent::TextDelta(String)`; this handler
//!      follows that precedent.
//!
//! Resolution per mission "Spec-reality drift → reconcile, adapt scope,
//! document in commit body" rule:
//!   - Ship `/extra-usage` as a **6-section reorganisation** of the
//!     existing session-stats data, with per-turn averages + cost/1k
//!     efficiency metrics added on top.
//!   - Defer the `MetricsRegistry`-aggregation work to a follow-up
//!     ticket (will require hoisting `MetricsRegistry::new()` out of
//!     `task.rs` into a session-shared `Arc` and threading it through
//!     `SlashCommandContext`).
//!   - The `NOTES` section in the rendered output explicitly flags
//!     that task-counter and provider-level aggregation are deferred,
//!     so the surface area is honest about the scope reduction.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/extra-usage` handler — emits a 6-section detailed usage report.
///
/// Reads `usage_snapshot` (populated by the shared `Some("usage") |
/// Some("extra-usage")` arm in `src/command/context.rs`). Pre-computed
/// owned values; no locks held during emit.
pub(crate) struct ExtraUsageHandler;

impl CommandHandler for ExtraUsageHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let snap = ctx.usage_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "ExtraUsageHandler invoked without usage_snapshot populated \
                 — build_command_context bug (extra-usage arm missing?)"
            )
        })?;

        let session_id_line = ctx
            .session_id
            .as_deref()
            .unwrap_or("(none)")
            .to_string();

        let total_tokens = snap.input_tokens.saturating_add(snap.output_tokens);

        // Per-turn averages — guard against zero turns (otherwise
        // would NaN/Inf division).
        let (tokens_per_turn_label, cost_per_turn_label) = if snap.turn_count == 0 {
            ("n/a".to_string(), "n/a".to_string())
        } else {
            let tpt = total_tokens as f64 / snap.turn_count as f64;
            let cpt = snap.total_cost / snap.turn_count as f64;
            (format!("{tpt:.0}"), format!("${cpt:.4}"))
        };

        // Cost per 1k tokens — guard against zero total tokens.
        let cost_per_1k_label = if total_tokens == 0 {
            "n/a".to_string()
        } else {
            let per_1k = snap.total_cost / (total_tokens as f64 / 1000.0);
            format!("${per_1k:.4}")
        };

        // 6 SECTIONS: SESSION, TOKENS, COSTS, CACHE, EFFICIENCY, NOTES.
        // Each header is a single uppercase word; sub-lines indented two
        // spaces. The cache_stats_line (already pre-computed in the
        // snapshot, three lines joined by \n) is inlined under CACHE
        // with consistent indentation.
        let cache_indented = snap
            .cache_stats_line
            .lines()
            .map(|l| format!("  {}", l))
            .collect::<Vec<_>>()
            .join("\n");

        let msg = format!(
            "\nDetailed usage report\n\
             ─────────────────────\n\
             SESSION\n\
             \x20 ID:           {session_id}\n\
             \x20 Turns:        {turns}\n\
             \n\
             TOKENS\n\
             \x20 Input:        {inp} tokens\n\
             \x20 Output:       {out} tokens\n\
             \x20 Total:        {tot} tokens\n\
             \n\
             COSTS\n\
             \x20 Input:        ${input_cost:.4}\n\
             \x20 Output:       ${output_cost:.4}\n\
             \x20 Total:        ${total:.4}\n\
             \x20 Per turn:     {cost_per_turn}\n\
             \n\
             CACHE\n\
             {cache_block}\n\
             \n\
             EFFICIENCY\n\
             \x20 Tokens/turn:  {tokens_per_turn}\n\
             \x20 Cost/1k tok:  {cost_per_1k}\n\
             \n\
             NOTES\n\
             \x20 Task-counter aggregation: deferred (no session-shared\n\
             \x20   MetricsRegistry — see /extra-usage module rustdoc).\n\
             \x20 Provider-level aggregation: deferred (per-call Usage is\n\
             \x20   accumulated into session_stats; per-provider breakdown\n\
             \x20   needs a follow-up).\n",
            session_id = session_id_line,
            turns = snap.turn_count,
            inp = snap.input_tokens,
            out = snap.output_tokens,
            tot = total_tokens,
            input_cost = snap.input_cost,
            output_cost = snap.output_cost,
            total = snap.total_cost,
            cost_per_turn = cost_per_turn_label,
            cache_block = cache_indented,
            tokens_per_turn = tokens_per_turn_label,
            cost_per_1k = cost_per_1k_label,
        );

        ctx.emit(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        "Show detailed session usage report (extends /usage with 6 grouped sections)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // No aliases — primary `/extra-usage` is the canonical surface.
        // `/usage` remains a separate primary for the compact view.
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn execute_without_snapshot_returns_err() {
        let handler = ExtraUsageHandler;
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err(), "expected Err when usage_snapshot is None");
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("usage_snapshot"),
            "error must reference usage_snapshot; got: {}",
            msg
        );
    }

    #[test]
    fn execute_emits_six_section_headers() {
        // Six distinct uppercase headers must appear in the rendered
        // output: SESSION, TOKENS, COSTS, CACHE, EFFICIENCY, NOTES.
        let handler = ExtraUsageHandler;
        let snap = fixture_usage_snapshot();
        let (mut ctx, mut rx) = make_usage_ctx(Some(snap));
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1, "expected one TextDelta; got: {:?}", events);
        let body = match &events[0] {
            TuiEvent::TextDelta(s) => s.clone(),
            other => panic!("expected TextDelta, got {:?}", other),
        };

        for header in [
            "SESSION",
            "TOKENS",
            "COSTS",
            "CACHE",
            "EFFICIENCY",
            "NOTES",
        ] {
            assert!(
                body.contains(header),
                "rendered output missing section header `{}`; full body:\n{}",
                header,
                body
            );
        }
    }

    #[test]
    fn execute_with_zero_turns_renders_n_a() {
        // Per-turn averages must NOT divide by zero — they must
        // render as the literal string "n/a".
        let handler = ExtraUsageHandler;
        let snap = crate::command::usage::UsageSnapshot {
            input_tokens: 0,
            output_tokens: 0,
            turn_count: 0,
            input_cost: 0.0,
            output_cost: 0.0,
            total_cost: 0.0,
            cache_stats_line: "Cache hit rate: 0.0% (0 reads / 0 total)\n\
                 Cache creation: 0 tokens\n\
                 Estimated savings: 0 token-equivalents"
                .to_string(),
        };
        let (mut ctx, mut rx) = make_usage_ctx(Some(snap));
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(
            body.contains("n/a"),
            "zero-turns output must contain `n/a`; full body:\n{}",
            body
        );
    }

    #[test]
    fn execute_includes_efficiency_metrics() {
        // EFFICIENCY section must include "Tokens/turn:" and
        // "Cost/1k tok:" labels — these are the new metrics that
        // distinguish /extra-usage from /usage.
        let handler = ExtraUsageHandler;
        let snap = fixture_usage_snapshot(); // 1M in, 500K out, 3 turns
        let (mut ctx, mut rx) = make_usage_ctx(Some(snap));
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Tokens/turn:"), "missing Tokens/turn label");
        assert!(body.contains("Cost/1k tok:"), "missing Cost/1k tok label");
    }

    #[test]
    fn description_mentions_extra_detail() {
        let desc = ExtraUsageHandler.description();
        assert!(!desc.is_empty());
        assert!(
            desc.to_lowercase().contains("detail")
                || desc.to_lowercase().contains("section")
                || desc.to_lowercase().contains("extend"),
            "description should reference detail/sections/extension; got: {}",
            desc
        );
    }

    #[test]
    fn aliases_empty() {
        // /extra-usage has no aliases — `/usage` remains a separate
        // primary for the compact view.
        assert_eq!(ExtraUsageHandler.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn extra_usage_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("extra-usage") must return Some,
        // and execute with a populated usage_snapshot must emit a
        // single TextDelta carrying ALL SIX section headers.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("extra-usage")
            .expect("extra-usage must be registered in default_registry()");

        let snap = fixture_usage_snapshot();
        let (mut ctx, mut rx) = make_usage_ctx(Some(snap));
        handler.execute(&mut ctx, &[]).unwrap();

        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1, "expected one TextDelta; got: {:?}", events);
        let body = match &events[0] {
            TuiEvent::TextDelta(s) => s.clone(),
            other => panic!("expected TextDelta, got {:?}", other),
        };
        for header in [
            "SESSION",
            "TOKENS",
            "COSTS",
            "CACHE",
            "EFFICIENCY",
            "NOTES",
        ] {
            assert!(
                body.contains(header),
                "registry-dispatched output missing section header `{}`",
                header
            );
        }
    }
}
