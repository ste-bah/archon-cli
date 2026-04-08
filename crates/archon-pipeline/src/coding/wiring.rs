//! Wiring Obligation Agent types and WiringPlan validation.
//!
//! Implements REQ-IMPROVE-003 (wiring obligation agent) and
//! REQ-IMPROVE-022 (Wiring Plan Approved Gate).

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::contract::{GateResult, TaskContract};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An explicit wiring plan produced by the wiring-obligation-agent.
///
/// Contains typed obligations that describe how new code integrates
/// with the existing codebase. Persisted to disk so it survives
/// compaction and session resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiringPlan {
    pub task_id: String,
    pub obligations: Vec<WiringObligation>,
    /// ISO 8601 timestamp — set when the WiringPlanApprovedGate passes.
    pub validated_at: Option<String>,
}

/// A single wiring obligation with status tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiringObligation {
    pub id: String,
    pub file: String,
    pub action: WiringAction,
    pub line_context: String,
    pub mandatory: bool,
    /// References a WiringRequirement from the TaskContract
    /// in the format "module::entrypoint".
    pub maps_to_contract_wiring: Option<String>,
    pub status: ObligationStatus,
}

/// The kind of wiring change required.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WiringAction {
    AddModDecl,
    AddImport,
    RegisterRoute,
    AddConfigKey,
    AddSerdeDerive,
    Other(String),
}

/// Tracks whether a wiring obligation has been fulfilled.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ObligationStatus {
    Pending,
    Met,
    Failed,
}

// ---------------------------------------------------------------------------
// Coverage validation
// ---------------------------------------------------------------------------

/// Validate that every `required_wiring` entry in the contract has at
/// least one matching obligation in the wiring plan.
///
/// Matching uses the `maps_to_contract_wiring` field on obligations,
/// which should contain "module::entrypoint" referencing the contract's
/// WiringRequirement.
pub fn validate_coverage(
    contract: &TaskContract,
    plan: &WiringPlan,
) -> std::result::Result<(), Vec<String>> {
    let mut errors = Vec::new();

    for req in &contract.required_wiring {
        let key = format!("{}::{}", req.module, req.entrypoint);
        let covered = plan
            .obligations
            .iter()
            .any(|o| o.maps_to_contract_wiring.as_deref() == Some(&key));
        if !covered {
            errors.push(format!(
                "Contract wiring requirement '{}' (type: {:?}) has no matching obligation",
                key, req.wiring_type,
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

/// Gate that validates the WiringPlan covers all contract requirements
/// and all obligations have valid fields.
pub struct WiringPlanApprovedGate;

impl WiringPlanApprovedGate {
    pub fn check(contract: &TaskContract, plan: &WiringPlan) -> GateResult {
        let mut errors = Vec::new();

        // Validate obligation fields
        for ob in &plan.obligations {
            if ob.file.trim().is_empty() {
                errors.push(format!("Obligation {} has empty file", ob.id));
            }
            if ob.line_context.trim().is_empty() {
                errors.push(format!("Obligation {} has empty line_context", ob.id));
            }
        }

        // Validate coverage of contract wiring requirements
        if let Err(coverage_errors) = validate_coverage(contract, plan) {
            errors.extend(coverage_errors);
        }

        GateResult {
            passed: errors.is_empty(),
            errors,
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// Save a wiring plan to `<session_dir>/wiring-plan.json`.
pub fn save_wiring_plan(plan: &WiringPlan, session_dir: &Path) -> Result<()> {
    let path = session_dir.join("wiring-plan.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(plan)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Load a wiring plan from `<session_dir>/wiring-plan.json`.
pub fn load_wiring_plan(session_dir: &Path) -> Result<WiringPlan> {
    let path = session_dir.join("wiring-plan.json");
    let data = std::fs::read_to_string(&path)?;
    let plan: WiringPlan = serde_json::from_str(&data)?;
    Ok(plan)
}

// ===========================================================================
// Integration Verification (REQ-IMPROVE-004, REQ-IMPROVE-007)
// ===========================================================================

/// Status of a single verification check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VerificationStatus {
    Verified,
    Unmet,
    Skipped,
}

/// Result of verifying a single wiring obligation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObligationVerification {
    pub obligation_id: String,
    pub status: VerificationStatus,
    pub evidence: String,
    pub tool_used: String,
}

/// Complete verification report for all obligations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub task_id: String,
    pub results: Vec<ObligationVerification>,
    pub all_mandatory_met: bool,
    pub verified_at: String,
}

/// Tool-based integration verifier. Uses filesystem operations (read/grep),
/// never LLM self-attestation.
pub struct IntegrationVerifier;

impl IntegrationVerifier {
    /// Verify all obligations in a wiring plan using tool-based checks.
    pub fn verify(plan: &WiringPlan, project_root: &Path) -> VerificationReport {
        let mut results = Vec::new();

        for obligation in &plan.obligations {
            let verification = Self::verify_obligation(obligation, project_root);
            results.push(verification);
        }

        let all_mandatory_met = results.iter().all(|r| {
            let ob = plan.obligations.iter().find(|o| o.id == r.obligation_id);
            match ob {
                Some(o) if o.mandatory => r.status == VerificationStatus::Verified,
                Some(_) => true, // non-mandatory doesn't affect the gate
                None => true,
            }
        });

        VerificationReport {
            task_id: plan.task_id.clone(),
            results,
            all_mandatory_met,
            verified_at: now_iso8601(),
        }
    }

    fn verify_obligation(
        obligation: &WiringObligation,
        project_root: &Path,
    ) -> ObligationVerification {
        if !obligation.mandatory {
            return ObligationVerification {
                obligation_id: obligation.id.clone(),
                status: VerificationStatus::Skipped,
                evidence: "Non-mandatory obligation, verification deferred".into(),
                tool_used: "none".into(),
            };
        }

        match &obligation.action {
            WiringAction::AddModDecl => Self::verify_mod_decl(obligation, project_root),
            WiringAction::AddImport => Self::verify_import(obligation, project_root),
            WiringAction::RegisterRoute => Self::verify_route(obligation, project_root),
            WiringAction::AddConfigKey => Self::verify_config_key(obligation, project_root),
            WiringAction::AddSerdeDerive => Self::verify_serde_derive(obligation, project_root),
            WiringAction::Other(_) => Self::verify_generic(obligation, project_root),
        }
    }

    /// Read the target file and search for its content.
    fn read_file(file: &str, project_root: &Path) -> Option<String> {
        let path = project_root.join(file);
        std::fs::read_to_string(path).ok()
    }

    /// Verify that a `mod <stem>;` or `pub mod <stem>;` declaration is present.
    /// The stem is extracted from line_context (e.g. `pub mod widgets;` → `widgets`).
    fn verify_mod_decl(
        obligation: &WiringObligation,
        project_root: &Path,
    ) -> ObligationVerification {
        let content = match Self::read_file(&obligation.file, project_root) {
            Some(c) => c,
            None => {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Unmet,
                    evidence: format!("File not found: {}", obligation.file),
                    tool_used: "Read".into(),
                };
            }
        };

        // Extract the module stem from line_context like "pub mod widgets;" or "mod widgets;"
        let stem = obligation
            .line_context
            .split_whitespace()
            .find(|w| !matches!(*w, "pub" | "mod"))
            .map(|w| w.trim_end_matches(';'))
            .unwrap_or("")
            .to_string();

        for line in content.lines() {
            let trimmed = line.trim();
            // Match "mod <stem>;" or "pub mod <stem>;"
            if (trimmed == format!("mod {};", stem) || trimmed == format!("pub mod {};", stem))
                && !stem.is_empty()
            {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Verified,
                    evidence: trimmed.to_string(),
                    tool_used: "Read".into(),
                };
            }
        }

        ObligationVerification {
            obligation_id: obligation.id.clone(),
            status: VerificationStatus::Unmet,
            evidence: format!("mod {}; not found in {}", stem, obligation.file),
            tool_used: "Read".into(),
        }
    }

    /// Verify that a `use` import statement is present in the consuming file.
    fn verify_import(obligation: &WiringObligation, project_root: &Path) -> ObligationVerification {
        let content = match Self::read_file(&obligation.file, project_root) {
            Some(c) => c,
            None => {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Unmet,
                    evidence: format!("File not found: {}", obligation.file),
                    tool_used: "Read".into(),
                };
            }
        };

        // line_context should contain the import path e.g. "use crate::widgets::Button"
        let needle = obligation.line_context.trim().trim_end_matches(';');

        for line in content.lines() {
            let trimmed = line.trim().trim_end_matches(';');
            if trimmed == needle || trimmed.contains(needle) {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Verified,
                    evidence: line.trim().to_string(),
                    tool_used: "Read".into(),
                };
            }
        }

        ObligationVerification {
            obligation_id: obligation.id.clone(),
            status: VerificationStatus::Unmet,
            evidence: format!("Import '{}' not found in {}", needle, obligation.file),
            tool_used: "Read".into(),
        }
    }

    /// Verify that a route registration pattern is present (.route(, .get(, .post(, etc.).
    fn verify_route(obligation: &WiringObligation, project_root: &Path) -> ObligationVerification {
        let content = match Self::read_file(&obligation.file, project_root) {
            Some(c) => c,
            None => {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Unmet,
                    evidence: format!("File not found: {}", obligation.file),
                    tool_used: "Read".into(),
                };
            }
        };

        let route_patterns = [".route(", ".get(", ".post(", ".put(", ".delete(", ".patch("];

        for line in content.lines() {
            let trimmed = line.trim();
            if route_patterns.iter().any(|p| trimmed.contains(p)) {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Verified,
                    evidence: trimmed.to_string(),
                    tool_used: "Read".into(),
                };
            }
        }

        ObligationVerification {
            obligation_id: obligation.id.clone(),
            status: VerificationStatus::Unmet,
            evidence: format!("No route registration pattern found in {}", obligation.file),
            tool_used: "Read".into(),
        }
    }

    /// Verify that `#[derive(` containing both `Serialize` and `Deserialize` is present.
    fn verify_serde_derive(
        obligation: &WiringObligation,
        project_root: &Path,
    ) -> ObligationVerification {
        let content = match Self::read_file(&obligation.file, project_root) {
            Some(c) => c,
            None => {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Unmet,
                    evidence: format!("File not found: {}", obligation.file),
                    tool_used: "Read".into(),
                };
            }
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.contains("#[derive(")
                && trimmed.contains("Serialize")
                && trimmed.contains("Deserialize")
            {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Verified,
                    evidence: trimmed.to_string(),
                    tool_used: "Read".into(),
                };
            }
        }

        ObligationVerification {
            obligation_id: obligation.id.clone(),
            status: VerificationStatus::Unmet,
            evidence: format!(
                "#[derive(Serialize, Deserialize)] not found in {}",
                obligation.file
            ),
            tool_used: "Read".into(),
        }
    }

    /// Verify a config key loading pattern from line_context.
    fn verify_config_key(
        obligation: &WiringObligation,
        project_root: &Path,
    ) -> ObligationVerification {
        Self::verify_generic(obligation, project_root)
    }

    /// Generic: read file and check if line_context text appears.
    fn verify_generic(
        obligation: &WiringObligation,
        project_root: &Path,
    ) -> ObligationVerification {
        let content = match Self::read_file(&obligation.file, project_root) {
            Some(c) => c,
            None => {
                return ObligationVerification {
                    obligation_id: obligation.id.clone(),
                    status: VerificationStatus::Unmet,
                    evidence: format!("File not found: {}", obligation.file),
                    tool_used: "Read".into(),
                };
            }
        };

        let needle = obligation.line_context.trim();
        if content.contains(needle) {
            ObligationVerification {
                obligation_id: obligation.id.clone(),
                status: VerificationStatus::Verified,
                evidence: format!("Found: {}", needle),
                tool_used: "Read".into(),
            }
        } else {
            ObligationVerification {
                obligation_id: obligation.id.clone(),
                status: VerificationStatus::Unmet,
                evidence: format!("'{}' not found in {}", needle, obligation.file),
                tool_used: "Read".into(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// WiringVerificationGate
// ---------------------------------------------------------------------------

/// Gate that blocks the pipeline if any mandatory wiring obligation is unmet.
pub struct WiringVerificationGate;

impl WiringVerificationGate {
    pub fn check(report: &VerificationReport) -> GateResult {
        if report.all_mandatory_met {
            GateResult {
                passed: true,
                errors: vec![],
            }
        } else {
            let errors: Vec<String> = report
                .results
                .iter()
                .filter(|r| r.status == VerificationStatus::Unmet)
                .map(|r| format!("Obligation {} unmet: {}", r.obligation_id, r.evidence))
                .collect();
            GateResult {
                passed: false,
                errors,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// Save a verification report to `<session_dir>/verification-report.json`.
pub fn save_verification_report(report: &VerificationReport, session_dir: &Path) -> Result<()> {
    let path = session_dir.join("verification-report.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Load a verification report from `<session_dir>/verification-report.json`.
pub fn load_verification_report(session_dir: &Path) -> Result<VerificationReport> {
    let path = session_dir.join("verification-report.json");
    let data = std::fs::read_to_string(&path)?;
    let report: VerificationReport = serde_json::from_str(&data)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_iso8601() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}Z", dur.as_secs())
}
