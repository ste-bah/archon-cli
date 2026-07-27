use super::*;
const AHDM_UNIVERSE: &[&str] = &["ES", "NQ", "SPY", "QQQ", "BTCUSDT", "ETHUSDT"];
const AHDM_TIMEFRAMES: &[&str] = &["1W", "1D", "240", "60", "15"];
pub(super) fn ahdm_citations(
    generated_at: &str,
    coverage_gaps: Vec<String>,
    native_backtest_gate_passed: bool,
) -> serde_json::Value {
    let data_coverage_gate_passed = coverage_gaps.is_empty();
    serde_json::json!({
        "schema_version": "archon-ahdm-citations-v1",
        "artifact_kind": "kb_evidence_inventory_citations",
        "strategy_id": "AHDM-v1",
        "generated_at": generated_at,
        "promotion_policy": {
            "every_rule_requires_citation_or_hypothesis": true,
            "hypotheses_barred_from_promotion_until_cited": true,
            "confidence_is_score_not_probability": true,
            "no_live_trading": true,
            "diagnostic_artifacts_cannot_promote": true
        },
        "kb_priority": [
            "trading-hybrid-system",
            "trading-market-structure",
            "trading-risk-management",
            "trading-backtesting",
            "trading-execution",
            "trading-strategy-research",
            "trading-postmortems"
        ],
        "secondary_non_authoritative_kbs": ["trading-elliott-wave"],
        "acceptance_criteria": {
            "AC-AHDM-004": {
                "name": "evidence-risk input",
                "status": "implemented_fail_closed",
                "input": "kb_rule_inventory",
                "promotion_rule": "rules may promote only when cited; hypotheses remain research-only until cited"
            }
        },
        "evidence_risk_input": {
            "id": "AC-AHDM-004",
            "risk": "kb_evidence",
            "fail_closed_behavior": "missing citation or uncited hypothesis blocks promotion"
        },
        "rules": ahdm_rule_manifest(generated_at)["rules"].clone(),
        "citation_review": "cited rules include reviewed_source_excerpt=true and a concrete source excerpt; generic rule-inventory locators are invalid",
        "inventory_gate": {
            "status": "blocked_fail_closed",
            "promotion_allowed": false,
            "fail_closed_behavior": "KB inventory can be generated for audit, but cannot promote StrategySpec/readiness unless data coverage and citation gates pass",
            "required_prerequisites": {
                "citation_or_hypothesis_policy_passed": true,
                "data_coverage_gate_passed": data_coverage_gate_passed,
                "hypothesis_rules_absent": false,
                "native_backtest_gate_passed": native_backtest_gate_passed
            },
            "prerequisite_evidence": {
                "coverage_matrix_checked_before_rule_extraction": true,
                "native_backtest_gate_checked_before_rule_extraction": true
            }
        },
        "promotion_allowed": false,
        "coverage_gaps": coverage_gaps,
        "residual_gaps": ahdm_citation_residual_gaps(data_coverage_gate_passed, native_backtest_gate_passed, generated_at)
    })
}

fn ahdm_citation_residual_gaps(
    data_coverage_passed: bool,
    native_backtest_passed: bool,
    generated_at: &str,
) -> Vec<serde_json::Value> {
    let mut gaps = Vec::new();
    if !data_coverage_passed || !native_backtest_passed {
        gaps.push(residual_gap("GAP-AHDM-DATA-001", "data", "Trading Data Lake coverage or native backtest prerequisite is unavailable", "Promotion-oriented strategy work is refused until coverage and native backtest gates pass", generated_at));
    }
    gaps.push(residual_gap(
        "GAP-AHDM-KB-HYPOTHESIS-001",
        "kb",
        "At least one AHDM rule remains classified as hypothesis",
        "Hypothesis rules remain research-only and cannot satisfy promotion gates",
        generated_at,
    ));
    gaps
}

pub(super) fn ahdm_inventory_markdown(citations: &serde_json::Value, gaps: &[String]) -> String {
    let mut text = String::from(
        "# AHDM-v1 KB Rule Inventory\n\nRules are cited or explicitly marked hypothesis. Hypotheses are barred from promotion until cited. Confidence is a score, not a probability. Live trading is out of scope.\n\n## AC-AHDM-004 Evidence-Risk Input\n\n- input: `kb_rule_inventory`\n- risk: `kb_evidence`\n- fail_closed_behavior: missing citation or uncited hypothesis blocks promotion\n\n## Prioritized KBs\n\n",
    );
    if let Some(kbs) = citations
        .get("kb_priority")
        .and_then(serde_json::Value::as_array)
    {
        for kb in kbs {
            text.push_str(&format!("- {}\n", kb.as_str().unwrap_or("unknown")));
        }
    }
    text.push_str("\n## Rules\n\n");
    if let Some(rules) = citations.get("rules").and_then(serde_json::Value::as_array) {
        for rule in rules {
            let citation = rule.get("citation").and_then(serde_json::Value::as_object);
            let locator = citation
                .and_then(|citation| citation.get("locator"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("none");
            let excerpt = rule
                .get("source_excerpt")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("hypothesis; no reviewed source excerpt until cited");
            text.push_str(&format!(
                "- `{}` weight={} status={} promotion_allowed={} citation={} source_excerpt={}\n",
                rule["id"].as_str().unwrap_or("unknown"),
                rule["weight"],
                rule["status"],
                rule["promotion_allowed"],
                locator,
                excerpt
            ));
        }
    }
    text.push_str("\n## Inventory Gate\n\n");
    if let Some(gate) = citations
        .get("inventory_gate")
        .and_then(serde_json::Value::as_object)
    {
        text.push_str(&format!(
            "- status={}\n- promotion_allowed={}\n- fail_closed_behavior={}\n",
            gate.get("status").unwrap_or(&serde_json::Value::Null),
            gate.get("promotion_allowed")
                .unwrap_or(&serde_json::Value::Null),
            gate.get("fail_closed_behavior")
                .unwrap_or(&serde_json::Value::Null)
        ));
    }
    text.push_str("\n## Data Coverage Gaps\n\n");
    if gaps.is_empty() {
        text.push_str("- none recorded by coverage matrix\n");
    } else {
        for gap in gaps {
            text.push_str(&format!("- {gap}\n"));
        }
    }
    text.push_str("\n## Residual Gaps\n\n");
    if let Some(residual_gaps) = citations
        .get("residual_gaps")
        .and_then(serde_json::Value::as_array)
    {
        for gap in residual_gaps {
            text.push_str(&format!(
                "- `{}`: {}\n",
                gap["id"], gap["fail_closed_behavior"]
            ));
        }
    }
    text
}

pub(super) fn ahdm_strategy_spec(
    registry: &PersistentDatasetRegistry,
    generated_at: &str,
) -> serde_json::Value {
    let manifest = ahdm_rule_manifest(generated_at);
    let required_datasets = ahdm_dataset_refs(registry);
    let required_cell_count = AHDM_UNIVERSE.len() * AHDM_TIMEFRAMES.len();
    let covered_cell_count = ahdm_covered_cell_count(&required_datasets);
    let promotion_eligible = covered_cell_count == required_cell_count
        && required_datasets.len() == required_cell_count
        && required_datasets.iter().all(ahdm_dataset_ref_promotes);
    serde_json::json!({
        "schema": "archon-ahdm-strategy-spec-v1",
        "strategy_id": "AHDM-v1",
        "generated_at": generated_at,
        "confidence": {"type": "score", "score_not_probability": true, "sum_weights": 100, "no_trade_below": 0.55, "paper_consideration_min": 0.70, "paper_requires_backtest_gates": true},
        "initial_bias": {
            "components": manifest["rules"].clone(),
            "sum_weights": 100,
            "missing_evidence_behavior": "no_trade_fail_closed"
        },
        "entry_models": manifest["entry_models"].clone(),
        "datasets": required_datasets.clone(),
        "dataset_coverage_gate": {
            "reference_type": "registered_dataset_id_and_version",
            "promotion_requires_registered_native_validated_datasets": true,
            "diagnostic_or_degraded_datasets_allowed_for_promotion": false,
            "missing_refs_behavior": "no_trade_fail_closed",
            "promotion_eligible": promotion_eligible,
            "dataset_refs": required_datasets.clone(),
            "unavailable_refs": []
        },
        "risk": {"position_sizing": {"risk_fraction": 0.005, "max_fraction": 0.01, "formula": "min(account_equity*risk_fraction/abs(entry-stop), account_equity*max_fraction/entry)", "promotion_claim": false}, "missing_or_invalid_inputs": "no_trade_fail_closed", "live_trading": "out_of_scope"},
        "instrument_universe": ["ES", "NQ", "SPY", "QQQ", "BTCUSDT", "ETHUSDT"],
        "timeframe_stack": ["1W", "1D", "240", "60", "15"],
        "coverage_universe": {"id": "trading-core-v1", "instruments": AHDM_UNIVERSE, "timeframes": AHDM_TIMEFRAMES, "required_cells": required_cell_count, "available_cells": covered_cell_count, "promotion_eligible": promotion_eligible, "missing_refs_behavior": "no_trade_fail_closed"},
        "required_datasets": required_datasets,
        "dataset_reference_policy": {"reference_type": "registered_dataset_id_and_version", "loose_file_references_allowed": false, "missing_refs_behavior": "no_trade_fail_closed"},
        "daily_bias_formula": manifest["rules"].clone(),
        "confidence_scoring": {"type": "score_not_probability", "sum_weights": 100, "no_trade_below": 0.55, "paper_consideration_min": 0.70},
        "confidence_gate_semantics": {"confidence_is_score": true, "confidence_is_probability": false, "less_than_0_55": "no_trade", "from_0_55_to_less_than_0_70": "research_only_no_paper_consideration", "greater_than_or_equal_0_70": "paper_consideration_only_after_backtest_gates_pass", "backtest_gates_required_for_paper": true, "live_trading": "out_of_scope"},
        "invalidation_logic": "entry invalidates on missing required evidence, broken setup level, or data gate failure",
        "stop_logic": "deterministic setup invalidation stop from shared manifest; no live order placement",
        "tp_logic": {"tp1": "first planned objective", "tp2": "second planned objective", "tp3": "final planned objective"},
        "prd_section_27": {"strategy": "AHDM-v1", "bias_components": "exact weighted initial_bias.components sum to 100", "entry_models": ["liquidity_sweep_reversal", "trend_continuation_pullback", "range_mean_reversion"], "dataset_references": "registered dataset_id/version records only; loose files are not evidence", "confidence": "score, not probability", "no_trade_rule": "confidence < 0.55 or missing required evidence", "paper_gate": "confidence >= 0.70 and backtest gates passed", "live_trading": "out_of_scope"},
        "no_trade_filters": ["confidence < 0.55 -> no_trade", "missing required evidence", "coverage gap", "non-native or degraded dataset", "failed backtest gate"],
        "position_sizing": {"risk_fraction": 0.005, "max_fraction": 0.01, "formula": "min(account_equity*risk_fraction/abs(entry-stop), account_equity*max_fraction/entry)", "promotion_claim": false},
        "slippage_cost_assumptions": {"fees": "from BacktestConfig", "spread_bps": "from BacktestConfig", "slippage_bps": "from BacktestConfig", "market_impact_bps": "from BacktestConfig"},
        "data_quality_gates": ["registered dataset id/version", "native_interval=true", "production_eligible=true", "validation passed", "checksum match"],
        "promotion_gates": {"paper": ["confidence >= 0.70", "backtest gates passed", "no hypothesis rules used for promotion"], "live": "out_of_scope"},
        "paper_trading_readiness_gates": ["confidence >= 0.70", "backtest gates passed", "native backtest replayable", "Pine exploratory only", "adversarial review accepted"],
        "source_citations": "evidence/citations.json",
        "residual_gaps": [residual_gap("GAP-AHDM-SPEC-001", "strategy", "Uncited hypothesis rules are retained for research only", "Hypothesis rules cannot satisfy promotion gates", generated_at)],
        "shared_rule_manifest": manifest.clone(),
        "rule_manifest": manifest
    })
}

fn ahdm_dataset_refs(registry: &PersistentDatasetRegistry) -> Vec<serde_json::Value> {
    let records = registry
        .datasets
        .values()
        .filter(|record| record.native_interval && record.production_eligible)
        .collect::<Vec<_>>();
    let ahdm_count = records.iter().filter(|record| ahdm_cell(record)).count();
    records
        .into_iter()
        .filter(|record| {
            ahdm_count != AHDM_UNIVERSE.len() * AHDM_TIMEFRAMES.len() || ahdm_cell(record)
        })
        .map(|record| {
            serde_json::json!({
                "dataset_id": record.dataset_id,
                "version": record.version,
                "registry_ref": format!("{}:{}", record.dataset_id, record.version),
                "provider": record.provider,
                "canonical_instrument": record.symbol,
                "timeframe": record.timeframe,
                "native_interval": record.native_interval,
                "production_eligible": record.production_eligible,
                "status": record.status,
                "checksum": record.checksum,
                "coverage_start": record.coverage_start,
                "coverage_end": record.coverage_end,
                "manifest_path": record.manifest_path,
                "normalized_path": record.normalized_path,
                "raw_path": record.raw_path,
                "validation_path": record.validation_path,
            })
        })
        .collect()
}

fn ahdm_cell(record: &StoredDatasetRecord) -> bool {
    AHDM_UNIVERSE.contains(&record.symbol.as_str())
        && AHDM_TIMEFRAMES.contains(&record.timeframe.as_str())
}

fn ahdm_covered_cell_count(dataset_refs: &[serde_json::Value]) -> usize {
    dataset_refs
        .iter()
        .filter_map(|dataset_ref| {
            Some((
                dataset_ref.get("canonical_instrument")?.as_str()?,
                dataset_ref.get("timeframe")?.as_str()?,
            ))
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

fn ahdm_dataset_ref_promotes(dataset_ref: &serde_json::Value) -> bool {
    dataset_ref
        .get("native_interval")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && dataset_ref
            .get("production_eligible")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        && dataset_ref.get("status") == Some(&serde_json::json!(DatasetStatus::Healthy))
}

pub(super) fn ahdm_pine_source(kind: &str, manifest: &serde_json::Value) -> String {
    let manifest_hash = sha256_hex(manifest.to_string().as_bytes());
    let rule_comments = ahdm_pine_rule_comments(manifest);
    let declaration = if kind == "strategy" {
        "strategy(\"AHDM-v1 strategy\", overlay=true, pyramiding=0)"
    } else {
        "indicator(\"AHDM-v1 indicator\", overlay=true)"
    };
    let mut source = format!(
        "//@version=6\n// AHDM-v1 {kind}; exploratory only; fail closed; shared_manifest_hash={manifest_hash}\n// native_and_pine_parity_key={}\n{rule_comments}{declaration}\nraw_score = input.float(0.0, \"raw_score shared-rule confidence score, not probability\", minval=0.0, maxval=1.0)\nrule_bias = input.float(0.0, \"rule input: bias score\")\nrule_entry = input.float(close, \"rule input: entry_zone\")\nrisk_points = input.float(10.0, \"rule input: stop distance\", minval=0.01)\nconfidence_score = math.min(math.max(raw_score, 0.0), 1.0)\nno_trade_state = confidence_score < 0.55\nbias = rule_bias >= 0 ? 1 : -1\nentry_zone = rule_entry\nstop = bias > 0 ? entry_zone - risk_points : entry_zone + risk_points\ntp1 = bias > 0 ? entry_zone + risk_points : entry_zone - risk_points\ntp2 = bias > 0 ? entry_zone + risk_points * 2 : entry_zone - risk_points * 2\ntp3 = bias > 0 ? entry_zone + risk_points * 3 : entry_zone - risk_points * 3\nsizing_hint = no_trade_state ? 0.0 : math.min(confidence_score, 1.0)\nplot(confidence_score, title=\"confidence_score\")\nplotchar(no_trade_state, title=\"no_trade_state\", char=\"X\")\nplot(bias, title=\"bias\")\nplot(entry_zone, title=\"entry_zone\")\nplot(stop, title=\"stop\")\nplot(tp1, title=\"tp1\")\nplot(tp2, title=\"tp2\")\nplot(tp3, title=\"tp3\")\nplot(sizing_hint, title=\"sizing_hint\")\n",
        manifest
            .get("native_and_pine_parity_key")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("AHDM-v1/shared-rule-manifest")
    );
    if kind == "strategy" {
        source.push_str("if no_trade_state\n    strategy.close(\"AHDM-v1-long\")\n    strategy.close(\"AHDM-v1-short\")\nelse if bias > 0\n    strategy.entry(\"AHDM-v1-long\", strategy.long, qty=sizing_hint)\n    strategy.exit(\"AHDM-v1-long-exit\", \"AHDM-v1-long\", stop=stop, limit=tp1)\nelse\n    strategy.entry(\"AHDM-v1-short\", strategy.short, qty=sizing_hint)\n    strategy.exit(\"AHDM-v1-short-exit\", \"AHDM-v1-short\", stop=stop, limit=tp1)\n");
    }
    source
}

fn ahdm_pine_rule_comments(manifest: &serde_json::Value) -> String {
    let mut text = String::new();
    if let Some(rules) = manifest.get("rules").and_then(serde_json::Value::as_array) {
        for rule in rules {
            if let Some(id) = rule.get("id").and_then(serde_json::Value::as_str) {
                text.push_str("// shared_rule_id=");
                text.push_str(id);
                text.push('\n');
            }
        }
    }
    text
}

pub(super) fn sha256_hex(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (input.len() as u64) * 8;
    let mut msg = input.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap_or_default());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

pub(super) fn validate_ahdm_citations(citations: &serde_json::Value) -> Result<(), DataStoreError> {
    let Some(rules) = citations.get("rules").and_then(serde_json::Value::as_array) else {
        return Err(DataStoreError::InvalidMetadata(
            "AHDM citations missing rules".into(),
        ));
    };
    for rule in rules {
        let status = rule.get("status").and_then(serde_json::Value::as_str);
        let has_citation = rule
            .get("citation")
            .and_then(serde_json::Value::as_object)
            .is_some_and(citation_has_reviewed_excerpt);
        let promotion_allowed = rule
            .get("promotion_allowed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let valid = status == Some("cited") && has_citation
            || status == Some("hypothesis") && !promotion_allowed;
        if !valid {
            return Err(DataStoreError::InvalidMetadata(
                "AHDM rule violates citation or hypothesis promotion policy".into(),
            ));
        }
    }
    Ok(())
}

fn citation_has_reviewed_excerpt(citation: &serde_json::Map<String, serde_json::Value>) -> bool {
    let locator = citation
        .get("locator")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let excerpt = citation
        .get("excerpt")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    citation.get("reviewed_source_excerpt") == Some(&serde_json::Value::Bool(true))
        && !locator.trim().is_empty()
        && !locator.ends_with(":rule-inventory")
        && !excerpt.trim().is_empty()
}

pub(super) fn residual_gap(
    id: &str,
    area: &str,
    description: &str,
    fail_closed: &str,
    created_at: &str,
) -> serde_json::Value {
    serde_json::json!({"id": id, "area": area, "description": description, "impact": fail_closed, "fail_closed_behavior": fail_closed, "owner": "Archon", "created_at": created_at})
}
