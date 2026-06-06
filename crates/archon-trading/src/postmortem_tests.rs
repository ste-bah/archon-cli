use super::*;

#[test]
fn t_paper_04_promotion_requires_valid_postmortem() {
    assert_eq!(
        require_postmortem_for_promotion(None),
        Err(PostmortemError::PromotionBlocked)
    );
    assert!(require_postmortem_for_promotion(Some(&report())).is_ok());
}

#[test]
fn t_post_01_structured_report_updates_failure_patterns() {
    let mut registry = FailurePatternRegistry::default();
    registry
        .ingest_postmortem(&report())
        .expect("valid postmortem ingests");
    registry
        .ingest_postmortem(&report())
        .expect("duplicate ingest is idempotent");

    assert_eq!(registry.patterns().len(), 3);
    assert!(
        registry
            .patterns()
            .iter()
            .any(|item| item.source == FailurePatternSource::RiskEvent)
    );
    assert!(
        registry
            .patterns()
            .iter()
            .any(|item| item.source == FailurePatternSource::SpecDeviation)
    );
    assert!(
        registry
            .patterns()
            .iter()
            .any(|item| item.source == FailurePatternSource::Lesson)
    );
}

#[test]
fn a_post_01_registry_never_changes_live_limits() {
    let mut registry = FailurePatternRegistry::default();
    let error = registry
        .request_live_limit_change("paper-session-1", "daily_loss_limit=10%")
        .expect_err("postmortem registry is advisory only");

    assert_eq!(error, PostmortemError::LiveLimitChangeBlocked);
    assert_eq!(registry.blocked_live_limit_change_attempts().len(), 1);
}

fn report() -> SessionPostmortem {
    SessionPostmortem {
        session_id: "paper-session-1".to_string(),
        mode: SessionMode::Paper,
        strategy_ids: vec!["strategy-a".to_string()],
        trades: vec![TradeSummary {
            trade_id: "trade-1".to_string(),
            instrument: "SPY".to_string(),
            quantity: 1.0,
            realized_pnl: 12.5,
        }],
        realized_pnl: 12.5,
        risk_events: vec![RiskEventSummary {
            event_id: "risk-1".to_string(),
            control_id: "REQ-RISK-004".to_string(),
            decision: "blocked".to_string(),
            strategy_attributable: true,
        }],
        spec_f13_deviations: vec![SpecDeviation {
            spec_f13_rule: "exit-discipline".to_string(),
            observed: "late manual exit".to_string(),
            severity: DeviationSeverity::Warning,
        }],
        lessons: vec!["tighten paper runbook".to_string()],
        session_closed_unix_ms: 1_000,
        completed_unix_ms: 1_000 + POSTMORTEM_SLA_MS,
    }
}
