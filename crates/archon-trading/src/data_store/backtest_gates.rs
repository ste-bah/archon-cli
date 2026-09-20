use super::*;
use crate::data_lake::{
    BACKTEST_DATA_GATE_SCHEMA, BacktestDataGateReport, BacktestDatasetRef, BacktestGateDecision,
    BacktestGateIssue, BacktestRunMode, DIAGNOSTIC_CLASSIFICATION, PRODUCTION_CLASSIFICATION,
    REJECTED_CLASSIFICATION, backtest_gate_allows_candle_read,
};

impl TradingDataLake {
    pub fn load_ohlcv_for_backtest(
        &self,
        dataset_id: &str,
        version: &str,
    ) -> Result<StoredOhlcvDataset, DataStoreError> {
        let report = self.evaluate_backtest_dataset(
            BacktestDatasetRef {
                dataset_id: dataset_id.into(),
                version: version.into(),
            },
            BacktestRunMode::Production,
        )?;
        if !backtest_gate_allows_candle_read(&report) {
            return Err(gate_refusal(&report));
        }
        self.load_registered_ohlcv(dataset_id, version)
    }

    pub fn load_ohlcv_for_diagnostic(
        &self,
        dataset_id: &str,
        version: &str,
    ) -> Result<StoredOhlcvDataset, DataStoreError> {
        let report = self.evaluate_backtest_dataset(
            BacktestDatasetRef {
                dataset_id: dataset_id.into(),
                version: version.into(),
            },
            BacktestRunMode::ExploratoryDiagnostic,
        )?;
        if !backtest_gate_allows_candle_read(&report) {
            return Err(gate_refusal(&report));
        }
        self.load_registered_ohlcv(dataset_id, version)
    }

    pub fn evaluate_backtest_dataset(
        &self,
        reference: BacktestDatasetRef,
        mode: BacktestRunMode,
    ) -> Result<BacktestDataGateReport, DataStoreError> {
        let evaluated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if !reference.is_strict() {
            return Ok(gate_report(
                &reference,
                mode,
                vec![BacktestGateIssue::structural(
                    &reference,
                    "loose_file_reference",
                    "dataset id and version must be strict registry identity components",
                )],
                evaluated_at,
            ));
        }
        let registry = match self.load_registry_migration(false) {
            Ok(migration) => migration.registry,
            Err(error) => {
                return Ok(gate_report(
                    &reference,
                    mode,
                    vec![BacktestGateIssue::structural(
                        &reference,
                        "registry_invalid",
                        sanitized_error(&error),
                    )],
                    evaluated_at,
                ));
            }
        };
        let Some(record) = registry
            .datasets
            .get(&registry_key(&reference.dataset_id, &reference.version))
            .cloned()
        else {
            return Ok(gate_report(
                &reference,
                mode,
                vec![BacktestGateIssue::structural(
                    &reference,
                    "dataset_missing",
                    "exact dataset id and version are not registered",
                )],
                evaluated_at,
            ));
        };
        Ok(gate_report(
            &reference,
            mode,
            evaluate_registered_dataset(&self.root, &record, &reference),
            evaluated_at,
        ))
    }

    pub(super) fn evaluate_backtest_data_gate(
        &self,
        dataset_id: &str,
        version: &str,
        diagnostic: bool,
    ) -> Result<BacktestDataGateReport, DataStoreError> {
        let mode = if diagnostic {
            BacktestRunMode::ExploratoryDiagnostic
        } else {
            BacktestRunMode::Production
        };
        let report = self.evaluate_backtest_dataset(
            BacktestDatasetRef {
                dataset_id: dataset_id.into(),
                version: version.into(),
            },
            mode,
        )?;
        if diagnostic {
            // Diagnostic mode always returns the report so callers can inspect
            // overridden_issues regardless of structural or policy failures.
            Ok(report)
        } else if backtest_gate_allows_candle_read(&report) {
            Ok(report)
        } else if report.issues.len() == 1 && report.issues[0].code == "dataset_missing" {
            Err(DataStoreError::MissingDataset(registry_key(
                dataset_id, version,
            )))
        } else {
            Err(gate_refusal(&report))
        }
    }

    pub(super) fn load_registered_ohlcv(
        &self,
        dataset_id: &str,
        version: &str,
    ) -> Result<StoredOhlcvDataset, DataStoreError> {
        let registry = self.load_verified_registry()?;
        let record = registry
            .datasets
            .get(&registry_key(dataset_id, version))
            .cloned()
            .ok_or_else(|| DataStoreError::MissingDataset(registry_key(dataset_id, version)))?;
        verify_artifacts(&self.root, &record)?;
        let metadata = read_dataset_metadata(&self.root, &record)?;
        validate_metadata(&metadata)
            .map_err(|error| DataStoreError::InvalidMetadata(format!("{error:?}")))?;
        let bars = read_jsonl_bars(&self.root.join(&record.normalized_path))?;
        Ok(StoredOhlcvDataset {
            record,
            metadata,
            bars,
        })
    }
}

fn evaluate_registered_dataset(
    root: &Path,
    record: &StoredDatasetRecord,
    reference: &BacktestDatasetRef,
) -> Vec<BacktestGateIssue> {
    let mut issues = structural_record_issues(root, record, reference);
    if issues.is_empty() {
        match load_gate_dataset(root, record) {
            Ok(dataset) => append_policy_issues(root, record, reference, &dataset, &mut issues),
            Err(error) => issues.push(BacktestGateIssue::structural(
                reference,
                structural_code(&error),
                sanitized_error(&error),
            )),
        }
    }
    issues.sort();
    issues.dedup();
    issues
}

fn structural_record_issues(
    root: &Path,
    record: &StoredDatasetRecord,
    reference: &BacktestDatasetRef,
) -> Vec<BacktestGateIssue> {
    let mut issues = Vec::new();
    if record.dataset_id != reference.dataset_id || record.version != reference.version {
        issues.push(BacktestGateIssue::structural(
            reference,
            "dataset_identity_mismatch",
            "registry record identity differs from the requested identity",
        ));
    }
    if let Err(error) = verify_artifacts(root, record) {
        issues.push(BacktestGateIssue::structural(
            reference,
            structural_code(&error),
            sanitized_error(&error),
        ));
    }
    issues
}

fn append_policy_issues(
    root: &Path,
    record: &StoredDatasetRecord,
    reference: &BacktestDatasetRef,
    dataset: &StoredOhlcvDataset,
    issues: &mut Vec<BacktestGateIssue>,
) {
    append_metadata_policy_issues(reference, &dataset.metadata, issues);
    append_validation_policy_issue(root, record, reference, issues);
    let mut details = Vec::new();
    append_dataset_gate_issues(root, record, dataset, &mut details);
    issues.extend(
        details
            .into_iter()
            .filter(|message| !covered_by_specific_policy(message))
            .map(|message| {
                BacktestGateIssue::policy(reference, policy_issue_code(&message), message)
            }),
    );
}

fn append_metadata_policy_issues(
    reference: &BacktestDatasetRef,
    metadata: &DatasetMetadata,
    issues: &mut Vec<BacktestGateIssue>,
) {
    let checks = [
        (
            "metadata_provider_missing",
            metadata.provider.trim().is_empty(),
        ),
        (
            "metadata_timeframe_missing",
            metadata.timeframe.trim().is_empty(),
        ),
        (
            "metadata_provider_symbol_missing",
            metadata.provider_symbol.trim().is_empty(),
        ),
        (
            "metadata_session_missing",
            metadata.session.trim().is_empty(),
        ),
        (
            "metadata_adjustment_missing",
            metadata.adjustment.trim().is_empty(),
        ),
        ("native_interval_false", !metadata.native_interval),
        ("production_eligible_false", !metadata.production_eligible),
    ];
    issues.extend(
        checks
            .into_iter()
            .filter(|(_, failed)| *failed)
            .map(|(code, _)| {
                BacktestGateIssue::policy(
                    reference,
                    code,
                    format!("{code} policy requirement failed"),
                )
            }),
    );
}

fn append_validation_policy_issue(
    root: &Path,
    record: &StoredDatasetRecord,
    reference: &BacktestDatasetRef,
    issues: &mut Vec<BacktestGateIssue>,
) {
    let Ok(report) = read_json::<ValidationReport>(&root.join(&record.validation_path)) else {
        return;
    };
    let code = match report.status {
        ValidationStatus::Passed => return,
        ValidationStatus::Degraded => "validation_status_degraded",
        ValidationStatus::Failed => "validation_status_failed",
    };
    issues.push(BacktestGateIssue::policy(
        reference,
        code,
        format!("validation report status is {code}"),
    ));
}

fn covered_by_specific_policy(message: &str) -> bool {
    message.contains("validation status")
        || message.contains("production eligible")
        || message.contains("registry status")
        || message.contains("exact-native")
        || message.contains("lineage evidence")
}

fn gate_report(
    reference: &BacktestDatasetRef,
    mode: BacktestRunMode,
    mut issues: Vec<BacktestGateIssue>,
    evaluated_at: String,
) -> BacktestDataGateReport {
    issues.sort();
    issues.dedup();
    let structural = issues.iter().any(|issue| !issue.overrideable);
    let decision = if issues.is_empty() && mode == BacktestRunMode::Production {
        BacktestGateDecision::ProductionAllowed
    } else if mode == BacktestRunMode::ExploratoryDiagnostic && !structural {
        BacktestGateDecision::DiagnosticOnly
    } else {
        BacktestGateDecision::Rejected
    };
    let classification = match decision {
        BacktestGateDecision::ProductionAllowed => PRODUCTION_CLASSIFICATION,
        BacktestGateDecision::DiagnosticOnly => DIAGNOSTIC_CLASSIFICATION,
        BacktestGateDecision::Rejected => REJECTED_CLASSIFICATION,
    };
    BacktestDataGateReport {
        schema_version: BACKTEST_DATA_GATE_SCHEMA.into(),
        dataset_id: reference.dataset_id.clone(),
        version: reference.version.clone(),
        mode,
        decision,
        classification: classification.into(),
        diagnostic: mode == BacktestRunMode::ExploratoryDiagnostic,
        promotion_eligible: decision == BacktestGateDecision::ProductionAllowed,
        // Diagnostic mode always copies issues to overridden_issues so callers
        // can inspect what was overridden, matching the legacy behavior.
        overridden_issues: if mode == BacktestRunMode::ExploratoryDiagnostic {
            issues.clone()
        } else {
            Vec::new()
        },
        issues,
        evaluated_at,
    }
}

fn gate_refusal(report: &BacktestDataGateReport) -> DataStoreError {
    // Join all issue messages so CLI and test assertions that check for human-readable
    // text such as "below required production backtest minimum" continue to match.
    let details: Vec<&str> = report.issues.iter().map(|i| i.message.as_str()).collect();
    DataStoreError::InvalidMetadata(format!(
        "backtest data gate refused dataset {}:{}: {}",
        report.dataset_id,
        report.version,
        details.join("; ")
    ))
}

fn structural_code(error: &DataStoreError) -> &'static str {
    match error {
        DataStoreError::Json(_) => "validation_invalid",
        DataStoreError::InvalidPath => "artifact_path_escape",
        DataStoreError::Io(_) => "normalized_artifact_missing",
        DataStoreError::IncompleteArtifactContract(message) if message.contains("checksum") => {
            "checksum_mismatch"
        }
        DataStoreError::IncompleteArtifactContract(_) => "raw_artifact_missing",
        _ => "metadata_invalid",
    }
}

fn policy_issue_code(detail: &str) -> &'static str {
    if detail.contains("validation status") {
        "validation_status_failed"
    } else if detail.contains("native") || detail.contains("lineage") {
        "native_interval_false"
    } else if detail.contains("production eligible") || detail.contains("registry status") {
        "production_eligible_false"
    } else if detail.contains("checksum") {
        "checksum_mismatch"
    } else if detail.contains("minimum") {
        "coverage_policy"
    } else {
        "validation_status_degraded"
    }
}

fn load_gate_dataset(
    root: &Path,
    record: &StoredDatasetRecord,
) -> Result<StoredOhlcvDataset, DataStoreError> {
    Ok(StoredOhlcvDataset {
        record: record.clone(),
        metadata: read_dataset_metadata(root, record)?,
        bars: read_jsonl_bars(&root.join(&record.normalized_path))?,
    })
}

fn sanitized_error(error: &DataStoreError) -> String {
    match error {
        DataStoreError::Json(_) => "persisted JSON contract is invalid".into(),
        DataStoreError::Io(_) => "declared artifact is unreadable".into(),
        DataStoreError::InvalidPath => "declared artifact path escapes the project root".into(),
        DataStoreError::IncompleteArtifactContract(message) if message.contains("checksum") => {
            "current artifact checksum chain is inconsistent".into()
        }
        DataStoreError::IncompleteArtifactContract(message) => {
            format!("declared artifact contract is incomplete: {message}")
        }
        _ => "persisted dataset evidence is invalid".into(),
    }
}
