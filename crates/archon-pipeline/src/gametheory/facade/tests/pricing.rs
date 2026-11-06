use super::super::*;
use super::*;

#[test]
fn missing_usage_stays_unknown_with_explicit_pricing() {
    let llm = canned_specialist_llm();
    assert_eq!(
        costs::estimate_llm_cost_usd(&llm, "custom-model", None, Some(12)),
        None
    );
    assert_eq!(
        costs::estimate_llm_cost_usd(&llm, "custom-model", Some(12), None),
        None
    );
    assert_eq!(
        costs::estimate_llm_cost_usd(&llm, "custom-model", Some(0), Some(0)),
        Some(0.0)
    );
}

struct PriceClient(Option<[f64; 2]>);

#[async_trait]
impl LlmClient for PriceClient {
    fn resolve_model_alias(&self, _: &str) -> String {
        "configured-model".into()
    }

    fn estimate_cost_usd(&self, _: &str, input: u64, output: u64) -> Option<f64> {
        archon_core::config::model_policy::token_cost(self.0, input, output)
    }

    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> anyhow::Result<LlmResponse> {
        panic!("a model without pricing must never be dispatched")
    }
}

#[test]
fn budget_requires_the_resolved_model_price_before_any_calls() {
    let agents = vec!["nash-equilibrium-finder".into()];
    let error = costs::require_budget_prices(&PriceClient(None), &agents, 10.0).unwrap_err();
    assert!(error.to_string().contains("configured-model"));
    assert!(error.to_string().contains("llm.model_prices.\"<model>\""));
    assert!(error.to_string().contains("set the budget to unbounded"));
    assert!(costs::require_budget_prices(&PriceClient(Some([3.0, 15.0])), &agents, 10.0).is_ok());
    assert!(costs::require_budget_prices(&PriceClient(None), &agents, f64::INFINITY).is_ok());
}

#[test]
fn shipped_defaults_refuse_unknown_models_with_the_configuration_hint() {
    let config = archon_core::config::LlmConfig::default();
    let client = PriceClient(config.model_prices.get("configured-model").copied());
    let agents = vec!["nash-equilibrium-finder".into()];
    let error = costs::require_budget_prices(&client, &agents, 10.0).unwrap_err();
    assert!(error.to_string().contains(
        "Cannot enforce game-theory budget: model 'configured-model' has no valid price; set llm.model_prices.\"<model>\" (quote the key) for 'configured-model' or set the budget to unbounded"
    ));
}

#[test]
fn budgeted_execution_refuses_unpriced_model_before_dispatch() {
    let routing = RoutingDecision {
        run_id: "unpriced".into(),
        fingerprint_id: "unpriced".into(),
        enabled_specialists: vec!["nash-equilibrium-finder".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: String::new(),
    };
    let error = block_on(execute_specialists_real_with_options(
        &PriceClient(None),
        &routing,
        &test_fingerprint("unpriced"),
        "situation",
        &GameTheoryMemoryContext::default(),
        &test_options(),
        None,
    ))
    .err()
    .expect("unpriced model must refuse");
    assert!(error.to_string().contains("configured-model"));
}

#[test]
fn real_specialist_execution_keeps_unknown_usage_cost_unknown() {
    // Expected: real specialist execution with one unavailable usage direction
    // yields no cost estimate and records an unknown cost instead of $0.0.
    let llm = CapturingLlmClient::with_unknown_usage("specialist output");
    let routing = RoutingDecision {
        run_id: "run-unknown-cost".into(),
        fingerprint_id: "fp-unknown-cost".into(),
        enabled_specialists: vec!["nash-equilibrium-finder".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-01-01T00:00:00Z".into(),
    };
    let outcome = block_on(execute_specialists_real_with_options(
        &llm,
        &routing,
        &test_fingerprint("fp-unknown-cost"),
        "Two firms choose prices.",
        &GameTheoryMemoryContext::default(),
        &GameTheoryRunOptions {
            budget_usd: f64::INFINITY,
            ..super::test_options()
        },
        None,
    ))
    .unwrap();
    assert_eq!(outcome.costs_usd["nash-equilibrium-finder"], None);
    assert_eq!(outcome.unknown_cost_count, 1);
    assert_eq!(outcome.total_cost_usd, 0.0);
    eprintln!(
        "EVIDENCE #369 specialist ACTION: executed a successful specialist with tokens_in=None and tokens_out=Some(12); EXPECTED: unknown estimate; ACTUAL: cost=None, known subtotal={}, unknown specialists={}",
        outcome.total_cost_usd, outcome.unknown_cost_count
    );
}
