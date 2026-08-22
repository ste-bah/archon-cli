use serde::{Deserialize, Serialize};

pub const BACKTEST_DATA_GATE_SCHEMA: &str = "archon-backtest-data-gate-v1";
pub const PRODUCTION_CLASSIFICATION: &str = "production";
pub const DIAGNOSTIC_CLASSIFICATION: &str = "exploratory_diagnostic_non_promotable";
pub const REJECTED_CLASSIFICATION: &str = "rejected";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestDatasetRef {
    pub dataset_id: String,
    pub version: String,
}

impl BacktestDatasetRef {
    pub fn is_strict(&self) -> bool {
        strict_identity_component(&self.dataset_id) && strict_identity_component(&self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacktestRunMode {
    Production,
    ExploratoryDiagnostic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacktestGateDecision {
    ProductionAllowed,
    DiagnosticOnly,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacktestGateIssueClass {
    Structural,
    Policy,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestGateIssue {
    pub code: String,
    pub dataset_id: String,
    pub version: String,
    pub artifact_path: Option<String>,
    pub message: String,
    pub overrideable: bool,
    pub class: BacktestGateIssueClass,
}

impl BacktestGateIssue {
    pub fn structural(
        reference: &BacktestDatasetRef,
        code: &str,
        message: impl Into<String>,
    ) -> Self {
        Self::new(reference, code, message, false)
    }

    pub fn policy(reference: &BacktestDatasetRef, code: &str, message: impl Into<String>) -> Self {
        Self::new(reference, code, message, true)
    }

    pub fn at_artifact(mut self, path: impl Into<String>) -> Self {
        self.artifact_path = Some(path.into());
        self
    }

    pub fn contains(&self, value: &str) -> bool {
        self.code.contains(value) || self.message.contains(value)
    }

    fn new(
        reference: &BacktestDatasetRef,
        code: &str,
        message: impl Into<String>,
        overrideable: bool,
    ) -> Self {
        Self {
            code: code.into(),
            dataset_id: reference.dataset_id.clone(),
            version: reference.version.clone(),
            artifact_path: None,
            message: message.into(),
            overrideable,
            class: if overrideable {
                BacktestGateIssueClass::Policy
            } else {
                BacktestGateIssueClass::Structural
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestDataGateReport {
    pub schema_version: String,
    pub dataset_id: String,
    pub version: String,
    pub mode: BacktestRunMode,
    pub decision: BacktestGateDecision,
    pub classification: String,
    pub diagnostic: bool,
    pub promotion_eligible: bool,
    pub issues: Vec<BacktestGateIssue>,
    pub overridden_issues: Vec<BacktestGateIssue>,
    pub evaluated_at: String,
}

impl BacktestDataGateReport {
    pub fn has_structural_issues(&self) -> bool {
        self.issues.iter().any(|issue| !issue.overrideable)
    }

    pub fn is_consistent(&self) -> bool {
        self.schema_version == BACKTEST_DATA_GATE_SCHEMA
            && !self.evaluated_at.trim().is_empty()
            && self.issues_are_ordered_and_bound()
            && match self.decision {
                BacktestGateDecision::ProductionAllowed => self.is_production_allowed(),
                BacktestGateDecision::DiagnosticOnly => self.is_diagnostic_only(),
                BacktestGateDecision::Rejected => self.is_rejected(),
            }
    }

    fn is_production_allowed(&self) -> bool {
        self.mode == BacktestRunMode::Production
            && self.classification == PRODUCTION_CLASSIFICATION
            && !self.diagnostic
            && self.promotion_eligible
            && self.issues.is_empty()
            && self.overridden_issues.is_empty()
            && BacktestDatasetRef {
                dataset_id: self.dataset_id.clone(),
                version: self.version.clone(),
            }
            .is_strict()
    }

    fn is_diagnostic_only(&self) -> bool {
        self.mode == BacktestRunMode::ExploratoryDiagnostic
            && self.classification == DIAGNOSTIC_CLASSIFICATION
            && self.diagnostic
            && !self.promotion_eligible
            && !self.has_structural_issues()
            && self.overridden_issues == self.issues
    }

    fn is_rejected(&self) -> bool {
        self.classification == REJECTED_CLASSIFICATION
            && !self.promotion_eligible
            && !self.issues.is_empty()
            && self.overridden_issues.is_empty()
    }

    fn issues_are_ordered_and_bound(&self) -> bool {
        let ordered = self.issues.windows(2).all(|pair| pair[0] < pair[1]);
        ordered
            && self.issues.iter().all(|issue| {
                issue.dataset_id == self.dataset_id
                    && issue.version == self.version
                    && !issue.code.trim().is_empty()
                    && !issue.message.trim().is_empty()
                    && issue.overrideable == (issue.class == BacktestGateIssueClass::Policy)
            })
    }
}

pub fn backtest_gate_allows_candle_read(report: &BacktestDataGateReport) -> bool {
    report.is_consistent()
        && matches!(
            report.decision,
            BacktestGateDecision::ProductionAllowed | BacktestGateDecision::DiagnosticOnly
        )
}

pub fn backtest_gate_allows_promotion(report: &BacktestDataGateReport) -> bool {
    report.is_consistent() && report.decision == BacktestGateDecision::ProductionAllowed
}

fn strict_identity_component(value: &str) -> bool {
    !value.is_empty()
        && !value.eq_ignore_ascii_case("latest")
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
