use super::*;

pub(super) fn expected(root: &Path, unconfirmed: bool) -> Vec<(String, PathBuf)> {
    let rows = if unconfirmed {
        r#"AC-AHDM-004 Data/provider/coverage
AC-AHDM-004 Slippage/execution
AC-DL-001 .archon/trading-lab/data
AC-DL-001 registry.json
SUP-REQ-AHDM-006 /@version
SUP-REQ-AHDM-007 strategy.entry
SUP-REQ-AHDM-007 strategy.exit
SUP-REQ-AHDM-022 AHDM-v1/shared-rule-manifest"#
    } else {
        r#"AC-AHDM-001 crates/archon-trading/src/data_store/ahdm/strategy_spec_gate.rs
AC-AHDM-001 crates/archon-trading/src/data_store/data_store_ahdm_tests/strategy_spec.rs
AC-AHDM-002 crates/archon-trading/src/data_store/ahdm.rs
AC-AHDM-002 crates/archon-trading/src/data_store/ahdm_methods.rs
AC-AHDM-002 crates/archon-trading/src/data_store/data_store_ahdm_tests/pine.rs
AC-AHDM-002 crates/archon-trading/src/pine_lab/ahdm.rs
AC-AHDM-003 crates/archon-trading/tests/ahdm_backtest_evidence.rs
AC-AHDM-004 crates/archon-trading/src/data_store/ahdm_readiness/review_findings.rs
AC-AHDM-004 crates/archon-trading/src/data_store/ahdm_readiness.rs
AC-AHDM-004 crates/archon-trading/src/data_store/data_store_ahdm_tests/readiness_review.rs
AC-DL-002 crates/archon-trading/tests/capability_engine.rs
AC-DL-003 crates/archon-trading/tests/ingest_artifacts.rs
AC-DL-004 crates/archon-trading/tests/coverage_matrix_generation.rs
AC-DL-006 crates/archon-trading/src/data_store/validation_tests.rs
AC-DL-006 crates/archon-trading/tests/validation_report.rs
AC-DL-006 crates/archon-trading/tests/validation_rules.rs
AC-DL-006 validation_report/emit_revalidate
AC-DL-007 crates/archon-trading/tests/capability_engine.rs
AC-DL-007 crates/archon-trading/tests/capability_unavailable.rs
AC-DL-007 crates/archon-trading/tests/openbb_polygon_adapter_unavailable.rs
AC-DL-007 crates/archon-trading/tests/openbb_polygon_ingest_artifacts.rs
AC-DL-007 crates/archon-trading/tests/openbb_polygon_native_ingest.rs
AC-DL-007 crates/archon-trading/tests/provider_capability_types.rs
AC-DL-007 crates/archon-trading/tests/stooq_adapter_unavailable.rs
AC-DL-007 crates/archon-trading/tests/stooq_ingest_artifacts.rs
AC-DL-007 crates/archon-trading/tests/stooq_native_ingest.rs
AC-DL-007 crates/archon-trading/tests/tradingview_adapter_unavailable.rs
AC-DL-007 crates/archon-trading/tests/tradingview_ingest_artifacts.rs
AC-DL-007 crates/archon-trading/tests/yfinance_adapter_unavailable/ac_dl_007_pair.rs
AC-DL-007 crates/archon-trading/tests/yfinance_adapter_unavailable.rs
AC-DL-007 crates/archon-trading/tests/yfinance_fallback_ingest.rs
AC-DL-007 crates/archon-trading/tests/yfinance_ingest_artifacts.rs
SUP-REQ-AHDM-001 crates/archon-trading/src/data_store/ahdm.rs
SUP-REQ-AHDM-001 crates/archon-trading/src/data_store/ahdm_methods.rs
SUP-REQ-AHDM-001 crates/archon-trading/src/data_store/coverage_methods.rs
SUP-REQ-AHDM-001 crates/archon-trading/src/data_store/data_store_ahdm_tests/prerequisites.rs
SUP-REQ-AHDM-002 crates/archon-trading/src/data_store/ahdm.rs
SUP-REQ-AHDM-002 crates/archon-trading/src/data_store/ahdm_evidence.rs
SUP-REQ-AHDM-002 crates/archon-trading/src/data_store/data_store_ahdm_tests/prerequisites.rs
SUP-REQ-AHDM-002 crates/archon-trading/src/data_store/data_store_ahdm_tests.rs
SUP-REQ-AHDM-002 crates/archon-trading/tests/kb.rs
SUP-REQ-AHDM-003 crates/archon-trading/tests/kb.rs
SUP-REQ-AHDM-005 crates/archon-trading/src/ahdm_backtest/spec/resolution_gate.rs
SUP-REQ-AHDM-005 crates/archon-trading/src/ahdm_backtest/spec.rs
SUP-REQ-AHDM-005 crates/archon-trading/src/data_store/ahdm/spec_shapes.rs
SUP-REQ-AHDM-005 crates/archon-trading/src/data_store/data_store_ahdm_tests/strategy_spec.rs
SUP-REQ-AHDM-005 crates/archon-trading/tests/ahdm_backtest/common/mod.rs
SUP-REQ-AHDM-005 crates/archon-trading/tests/ahdm_backtest.rs
SUP-REQ-AHDM-006 crates/archon-trading/src/data_store/ahdm.rs
SUP-REQ-AHDM-007 crates/archon-trading/src/data_store/ahdm.rs
SUP-REQ-AHDM-007 crates/archon-trading/src/data_store/ahdm_methods.rs
SUP-REQ-AHDM-007 crates/archon-trading/src/data_store/data_store_ahdm_tests/pine.rs
SUP-REQ-AHDM-007 crates/archon-trading/src/pine_lab/ahdm.rs
SUP-REQ-AHDM-008 crates/archon-trading/tests/ahdm_backtest.rs
SUP-REQ-AHDM-008 crates/archon-trading/tests/backtest_gate_refusals.rs
SUP-REQ-AHDM-009 crates/archon-trading/src/data_store/ahdm_readiness.rs
SUP-REQ-AHDM-009 crates/archon-trading/src/data_store/data_store_ahdm_tests/readiness_review.rs
SUP-REQ-AHDM-009 crates/archon-trading/src/data_store/data_store_ahdm_tests.rs
SUP-REQ-AHDM-010 crates/archon-trading/src/data_store/ahdm_readiness/blocked_review.rs
SUP-REQ-AHDM-010 crates/archon-trading/src/data_store/ahdm_readiness.rs
SUP-REQ-AHDM-010 crates/archon-trading/src/data_store/data_store_ahdm_tests/readiness_review.rs
SUP-REQ-AHDM-020 crates/archon-trading/src/data_store/ahdm.rs
SUP-REQ-AHDM-020 crates/archon-trading/src/data_store/ahdm_evidence.rs
SUP-REQ-AHDM-020 crates/archon-trading/src/data_store/data_store_ahdm_tests.rs
SUP-REQ-AHDM-021 crates/archon-trading/src/ahdm_backtest/spec/reference_gate.rs
SUP-REQ-AHDM-021 crates/archon-trading/src/ahdm_backtest/spec.rs
SUP-REQ-AHDM-021 crates/archon-trading/src/data_store/ahdm/strategy_spec_gate.rs
SUP-REQ-AHDM-021 crates/archon-trading/src/data_store/ahdm.rs
SUP-REQ-AHDM-021 crates/archon-trading/src/promotion.rs
SUP-REQ-AHDM-021 prds/PRD-TRADING-DATA-LAKE-AHDM-001.md
SUP-REQ-AHDM-022 crates/archon-trading/src/data_store/ahdm/spec_shapes.rs
SUP-REQ-AHDM-022 crates/archon-trading/src/data_store/ahdm.rs
SUP-REQ-AHDM-022 crates/archon-trading/src/data_store/ahdm_evidence.rs
SUP-REQ-AHDM-022 crates/archon-trading/src/data_store/ahdm_methods.rs
SUP-REQ-AHDM-023 crates/archon-trading/src/data_store/ahdm_readiness.rs
SUP-REQ-AHDM-023 crates/archon-trading/src/data_store/data_store_ahdm_tests/pine.rs
SUP-REQ-AHDM-023 crates/archon-trading/src/pine_lab/ahdm.rs
SUP-REQ-AHDM-023 crates/archon-trading/src/pine_lab/ahdm_tests/capture_tests.rs
SUP-REQ-AHDM-024 crates/archon-trading/src/ahdm_backtest/spec.rs
SUP-REQ-AHDM-024 crates/archon-trading/src/data_lake/backtest_gate_contracts.rs
SUP-REQ-AHDM-024 crates/archon-trading/tests/ahdm_backtest_evidence.rs
SUP-REQ-AHDM-024 crates/archon-trading/tests/backtest.rs
SUP-REQ-AHDM-024 crates/archon-trading/tests/backtest_report_replay.rs
SUP-REQ-AHDM-030 crates/archon-trading/src/ahdm_backtest/report.rs
SUP-REQ-AHDM-030 crates/archon-trading/src/ahdm_backtest/spec/resolution_gate.rs
SUP-REQ-AHDM-030 crates/archon-trading/src/data_store/ahdm/spec_shapes.rs
SUP-REQ-AHDM-030 crates/archon-trading/src/data_store/ahdm_readiness.rs
SUP-REQ-AHDM-031 crates/archon-trading/tests/ahdm_backtest.rs
SUP-REQ-AHDM-031 crates/archon-trading/tests/ahdm_backtest_evidence.rs
SUP-REQ-AHDM-032 crates/archon-trading/src/ahdm_backtest/engine.rs
SUP-REQ-AHDM-032 crates/archon-trading/src/ahdm_backtest/spec/resolution_gate.rs
SUP-REQ-AHDM-032 crates/archon-trading/tests/ahdm_backtest_evidence.rs
SUP-REQ-AHDM-033 crates/archon-trading/src/ahdm_backtest/report.rs
SUP-REQ-AHDM-033 crates/archon-trading/tests/ahdm_backtest_evidence.rs
SUP-REQ-BT-001 crates/archon-trading/tests/ahdm_backtest.rs
SUP-REQ-BT-001 crates/archon-trading/tests/backtest_report_replay.rs
SUP-REQ-BT-002 crates/archon-trading/tests/ahdm_backtest_evidence.rs
SUP-REQ-BT-002 crates/archon-trading/tests/backtest_data_gates.rs
SUP-REQ-BT-002 crates/archon-trading/tests/backtest_diagnostic_override.rs
SUP-REQ-BT-002 crates/archon-trading/tests/backtest_gate_refusals.rs
SUP-REQ-BT-003 crates/archon-trading/tests/backtest_diagnostic_override.rs
SUP-REQ-BT-003 crates/archon-trading/tests/backtest_report_replay.rs
SUP-REQ-BT-004 crates/archon-trading/src/backtest_gates/run_record.rs
SUP-REQ-BT-004 crates/archon-trading/tests/ahdm_backtest.rs
SUP-REQ-BT-004 crates/archon-trading/tests/backtest_report_replay.rs
SUP-REQ-DL-004 crates/archon-trading/tests/ingest_artifacts.rs
SUP-REQ-DL-004 crates/archon-trading/tests/registry_migration_v1.rs
SUP-REQ-DL-004 crates/archon-trading/tests/registry_v2.rs
SUP-REQ-DL-010 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-010 crates/archon-trading/src/data_store/tests/metadata_artifact_gaps.rs
SUP-REQ-DL-010 crates/archon-trading/tests/ingest_artifacts.rs
SUP-REQ-DL-011 crates/archon-trading/src/data_lake/tradingview_mcp.rs
SUP-REQ-DL-011 crates/archon-trading/src/data_store/tradingview_ingest.rs
SUP-REQ-DL-011 crates/archon-trading/src/data_store/tradingview_ingest_tests.rs
SUP-REQ-DL-012 crates/archon-trading/src/data_lake/model.rs
SUP-REQ-DL-012 crates/archon-trading/src/data_lake/tests/artifact_tolerance.rs
SUP-REQ-DL-012 crates/archon-trading/src/data_store/openbb_polygon_ingest.rs
SUP-REQ-DL-012 crates/archon-trading/src/data_store/stooq_ingest.rs
SUP-REQ-DL-012 crates/archon-trading/src/data_store/yfinance_ingest.rs
SUP-REQ-DL-012 crates/archon-trading/src/providers/openbb_polygon_store.rs
SUP-REQ-DL-012 crates/archon-trading/src/providers/stooq_store.rs
SUP-REQ-DL-012 crates/archon-trading/src/providers/yfinance_store.rs
SUP-REQ-DL-012 crates/archon-trading/tests/stooq_ingest_artifacts.rs
SUP-REQ-DL-013 crates/archon-trading/src/data_store/yfinance_ingest.rs
SUP-REQ-DL-013 crates/archon-trading/src/data_store/yfinance_ingest_tests.rs
SUP-REQ-DL-013 crates/archon-trading/src/providers/yfinance_store.rs
SUP-REQ-DL-013 crates/archon-trading/tests/yfinance_ingest_artifacts/crypto_provenance.rs
SUP-REQ-DL-013 crates/archon-trading/tests/yfinance_ingest_artifacts/providers_lane_crypto_provenance.rs
SUP-REQ-DL-013 crates/archon-trading/tests/yfinance_ingest_artifacts.rs
SUP-REQ-DL-031 crates/archon-trading/src/data_lake/provider_capability.rs
SUP-REQ-DL-031 crates/archon-trading/src/data_lake/provider_capability_tests.rs
SUP-REQ-DL-031 crates/archon-trading/tests/openbb_polygon_adapter_mapping.rs
SUP-REQ-DL-031 crates/archon-trading/tests/openbb_polygon_adapter_unavailable.rs
SUP-REQ-DL-031 crates/archon-trading/tests/provider_capability_types.rs
SUP-REQ-DL-031 crates/archon-trading/tests/stooq_adapter_mapping.rs
SUP-REQ-DL-031 crates/archon-trading/tests/stooq_adapter_unavailable.rs
SUP-REQ-DL-031 crates/archon-trading/tests/tradingview_adapter_mapping.rs
SUP-REQ-DL-031 crates/archon-trading/tests/tradingview_adapter_unavailable.rs
SUP-REQ-DL-031 crates/archon-trading/tests/yfinance_adapter_mapping.rs
SUP-REQ-DL-031 crates/archon-trading/tests/yfinance_adapter_unavailable.rs
SUP-REQ-DL-032 crates/archon-trading/src/data_lake/provider_capability_tests.rs
SUP-REQ-DL-032 crates/archon-trading/tests/provider_capability_types.rs
SUP-REQ-DL-032 crates/archon-trading/tests/tradingview_adapter_unavailable.rs
SUP-REQ-DL-041 crates/archon-trading/tests/coverage_matrix_generation.rs
SUP-REQ-DL-041 crates/archon-trading/tests/coverage_selection_freshness.rs
SUP-REQ-DL-042 crates/archon-trading/src/data_store/coverage_tests.rs
SUP-REQ-DL-042 crates/archon-trading/tests/coverage_selection_freshness.rs
SUP-REQ-DL-056 crates/archon-trading/src/validation/real_root.rs
SUP-REQ-DL-056 crates/archon-trading/tests/validation_report/emit_revalidate.rs
SUP-REQ-DL-056 crates/archon-trading/tests/validation_report.rs
SUP-REQ-DL-056 validation_report/emit_revalidate
SUP-REQ-DL-060 src/cli_args/tests.rs
SUP-REQ-DL-060 src/cli_args/tests_trading_data/fetch_native.rs
SUP-REQ-DL-060 src/cli_args/tests_trading_data.rs
SUP-REQ-DL-060 src/command/trading_data_registry_read_tests.rs
SUP-REQ-DL-060 src/command/trading_data_tests/metadata_contract.rs
SUP-REQ-DL-060 src/command/trading_data_tests/snapshot_target_tests.rs
SUP-REQ-DL-060 src/command/trading_data_tests.rs
SUP-REQ-DL-061 src/cli_args/tests_trading_data.rs
SUP-REQ-DL-061 src/command/trading_data_tests/metadata_contract.rs
SUP-REQ-DL-061 src/command/trading_data_tests/snapshot_target_tests.rs
SUP-REQ-DL-061 src/command/trading_data_tests.rs
SUP-REQ-DL-062 crates/archon-trading/tests/data_commands_surface.rs
SUP-REQ-DL-070 crates/archon-trading/src/data_lake/tests.rs
SUP-REQ-DL-071 crates/archon-trading/src/data_lake/identity.rs
SUP-REQ-DL-071 crates/archon-trading/src/data_lake/tests.rs
SUP-REQ-DL-072 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-072 crates/archon-trading/tests/dataset_identity_metadata_v1.rs
SUP-REQ-DL-073 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-073 crates/archon-trading/tests/ingest_artifacts.rs
SUP-REQ-DL-074 crates/archon-trading/src/data_lake/identity.rs
SUP-REQ-DL-074 crates/archon-trading/src/data_lake/tests.rs
SUP-REQ-DL-080 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-080 crates/archon-trading/tests/ingest_artifacts.rs
SUP-REQ-DL-081 crates/archon-trading/src/data_store/records.rs
SUP-REQ-DL-081 crates/archon-trading/src/data_store/tests/metadata_artifact_gaps.rs
SUP-REQ-DL-081 crates/archon-trading/src/data_store/util.rs
SUP-REQ-DL-081 crates/archon-trading/src/validation/checks.rs
SUP-REQ-DL-081 crates/archon-trading/tests/ingest_artifacts.rs
SUP-REQ-DL-082 crates/archon-trading/src/data_lake/tests.rs
SUP-REQ-DL-082 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-082 crates/archon-trading/src/data_store/validation_tests/security_contract.rs
SUP-REQ-DL-082 crates/archon-trading/src/providers/capability_tests/engine_tests.rs
SUP-REQ-DL-083 crates/archon-trading/src/backtest_gates/catalog.rs
SUP-REQ-DL-083 crates/archon-trading/src/data_lake/tests.rs
SUP-REQ-DL-083 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-084 crates/archon-trading/src/backtest_gates/catalog.rs
SUP-REQ-DL-084 crates/archon-trading/src/data_lake/tests.rs
SUP-REQ-DL-084 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-085 crates/archon-trading/src/data_lake/tests.rs
SUP-REQ-DL-085 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-085 crates/archon-trading/tests/registry_v2.rs
SUP-REQ-DL-090 crates/archon-trading/src/data_store/validation_tests/contract_core.rs
SUP-REQ-DL-090 crates/archon-trading/tests/validation_rules.rs
SUP-REQ-DL-103 crates/archon-trading/src/backtest_gates/catalog.rs
SUP-REQ-DL-103 crates/archon-trading/src/data_lake/validation_gate.rs
SUP-REQ-DL-103 crates/archon-trading/src/validation/gates.rs
SUP-REQ-DL-103 crates/archon-trading/tests/ahdm_backtest_evidence.rs
SUP-REQ-DL-103 crates/archon-trading/tests/backtest_diagnostic_override.rs
SUP-REQ-DL-103 crates/archon-trading/tests/backtest_gate_refusals.rs
SUP-REQ-DL-110 crates/archon-trading/src/data_lake/provider_capability_tests.rs
SUP-REQ-DL-110 crates/archon-trading/src/data_lake/stooq_tests.rs
SUP-REQ-DL-110 crates/archon-trading/src/providers/capability/report.rs
SUP-REQ-DL-110 crates/archon-trading/src/providers/capability_tests/engine_tests.rs
SUP-REQ-DL-110 crates/archon-trading/tests/capability_engine.rs
SUP-REQ-DL-110 crates/archon-trading/tests/capability_unavailable.rs
SUP-REQ-DL-111 crates/archon-trading/src/data_lake/provider_capability_tests.rs
SUP-REQ-DL-111 crates/archon-trading/src/data_lake/stooq_tests.rs
SUP-REQ-DL-111 crates/archon-trading/src/providers/capability/report.rs
SUP-REQ-DL-111 crates/archon-trading/src/providers/capability_tests/engine_tests.rs
SUP-REQ-DL-111 crates/archon-trading/tests/capability_unavailable.rs
SUP-REQ-DL-120 src/command/trading_data_registry_read_tests.rs
SUP-REQ-DL-120 src/command/trading_data_tests/metadata_contract.rs
SUP-REQ-DL-120 src/command/trading_data_tests.rs
SUP-REQ-DL-121 src/cli_args/tests_trading_data.rs
SUP-REQ-DL-121 src/command/trading_data_tests/metadata_contract.rs
SUP-REQ-DL-121 src/command/trading_data_tests/snapshot_target_tests.rs
SUP-REQ-DL-121 src/command/trading_data_tests.rs
SUP-REQ-DL-122 crates/archon-trading/src/data_store/data_store_ahdm_tests.rs
SUP-REQ-DL-122 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-122 crates/archon-trading/src/data_store/types.rs
SUP-REQ-DL-122 crates/archon-trading/tests/data_commands_surface/store_request_fixture.rs
SUP-REQ-DL-122 crates/archon-trading/tests/data_commands_surface.rs
SUP-REQ-DL-123 src/command/trading_data_tests.rs
SUP-REQ-DL-124 crates/archon-trading/tests/data_commands_surface.rs
SUP-REQ-DL-130 crates/archon-trading/tests/migration_v1_to_v2.rs
SUP-REQ-DL-130 crates/archon-trading/tests/validation_report/emit_revalidate.rs
SUP-REQ-DL-130 validation_report/emit_revalidate
SUP-REQ-DL-131 crates/archon-trading/src/data_lake/metadata_gate.rs
SUP-REQ-DL-131 crates/archon-trading/src/data_lake/production_gate.rs
SUP-REQ-DL-131 crates/archon-trading/src/data_store/data_store_tests.rs
SUP-REQ-DL-131 crates/archon-trading/tests/migration_v1_to_v2.rs
SUP-REQ-DL-132 crates/archon-trading/tests/migration_v1_to_v2.rs
SUP-REQ-DL-132 crates/archon-trading/tests/registry_v2.rs
SUP-REQ-DL-133 crates/archon-trading/tests/migration_v1_to_v2.rs
SUP-REQ-DL-133 crates/archon-trading/tests/registry_migration_v1.rs"#
    };
    let mut paths: Vec<_> = rows
        .lines()
        .map(|row| {
            let (check, path) = row.split_once(' ').unwrap();
            let expected = if Path::new(path).has_root() {
                Path::new(path).components().collect()
            } else {
                path.split('/')
                    .fold(root.to_path_buf(), |base, part| base.join(part))
            };
            (check.to_string(), expected)
        })
        .collect();
    sort(&mut paths);
    paths
}

// Compare path components, including roots, rather than rendered separators.
pub(super) fn sort(paths: &mut [(String, PathBuf)]) {
    paths.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.components().cmp(right.1.components()))
    });
}

#[test]
fn escaped_expression_fragments_are_not_rooted_reads() {
    let repo = repository();
    let root = repo.path();
    for command in [
        r#"inspect 'manifest(generated_at)\["rules"\]'"#,
        r#"inspect 'adjustment \\n policy recorded'"#,
    ] {
        let c = contract(command);
        let reads = input_discovery_for_check(&c.acceptance[0], root, root, None);
        assert!(
            reads.absolute_reads.is_empty(),
            "{command}: {:?}",
            reads.absolute_reads
        );
        assert!(
            reads.unconfirmed_reads.is_empty(),
            "{command}: {:?}",
            reads.unconfirmed_reads
        );
        let d = discover(&skeleton("owned.json"), &c, root, Some(root), root);
        assert!(d.unconfirmed.is_empty(), "{command}: {d:?}");
        assert!(d.outside.is_empty(), "{command}: {d:?}");
        assert!(d.unowned.is_empty(), "{command}: {d:?}");
    }
}

#[test]
fn rooted_operands_keep_component_identity() {
    let repo = repository();
    let root = repo.path();
    let c = contract("inspect '/@version'");
    let d = discover(&skeleton("owned.json"), &c, root, Some(root), root);
    assert_eq!(
        d.unconfirmed,
        [("AC-X-001".into(), absolute(root, "/@version"))]
    );
    assert_eq!(
        d.unconfirmed[0].1.components().collect::<Vec<_>>(),
        Path::new("/@version").components().collect::<Vec<_>>()
    );
}

#[cfg(windows)]
#[test]
fn complete_rooted_backslash_operand_is_still_observed() {
    let repo = repository();
    let root = repo.path();
    let c = contract(r"inspect '\inputs\value.json'");
    let d = discover(&skeleton("owned.json"), &c, root, Some(root), root);
    assert_eq!(
        d.unconfirmed,
        [("AC-X-001".into(), PathBuf::from(r"\inputs\value.json"))]
    );
}
