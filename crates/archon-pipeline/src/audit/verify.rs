use std::fs;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::audit::store::{PipelineBundleStore, sha256_hex};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationFinding {
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleVerificationReport {
    pub session_id: String,
    pub valid: bool,
    pub checked_at: String,
    pub manifest_present: bool,
    pub state_present: bool,
    pub audit_events: usize,
    pub agent_records: usize,
    pub findings: Vec<VerificationFinding>,
}

pub fn verify_bundle(
    store: &PipelineBundleStore,
    session_id: &str,
    write_report: bool,
) -> Result<BundleVerificationReport> {
    let dir = store.bundle_dir(session_id);
    let mut findings = Vec::new();

    let manifest_present = store.load_manifest(session_id).is_ok();
    if !manifest_present {
        findings.push(error("manifest.json missing or invalid"));
    }

    let state_present = store.load_state(session_id).is_ok();
    if !state_present {
        findings.push(error("state.json missing or checksum-invalid"));
    }

    let audit_path = dir.join("audit.log");
    let mut audit_events = 0usize;
    match fs::read_to_string(&audit_path) {
        Ok(raw) => {
            for (idx, line) in raw.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                audit_events += 1;
                if serde_json::from_str::<serde_json::Value>(line).is_err() {
                    findings.push(error(format!("audit.log line {} is invalid JSON", idx + 1)));
                }
            }
        }
        Err(_) => findings.push(error("audit.log missing or unreadable")),
    }

    let agents = store.list_agent_records(session_id).unwrap_or_else(|e| {
        findings.push(error(format!("agent records unreadable: {e}")));
        Vec::new()
    });

    let mut seen = std::collections::HashSet::new();
    for record in &agents {
        if !seen.insert(record.agent_key.clone()) {
            findings.push(error(format!(
                "duplicate agent record for {}",
                record.agent_key
            )));
        }
        let output_path = dir.join(&record.output_path);
        match fs::read(&output_path) {
            Ok(bytes) => {
                let actual = sha256_hex(&bytes);
                if actual != record.output_hash {
                    findings.push(error(format!(
                        "output hash mismatch for {}",
                        record.agent_key
                    )));
                }
            }
            Err(_) => findings.push(error(format!(
                "output file missing for {}",
                record.agent_key
            ))),
        }
        if !dir.join(&record.prompt_record_path).exists() {
            findings.push(warn(format!(
                "prompt record missing for {}",
                record.agent_key
            )));
        }
        for attempt in &record.attempts {
            let Some(output_path) = &attempt.output_path else {
                findings.push(warn(format!(
                    "attempt {} for {} has no output artifact",
                    attempt.attempt, record.agent_key
                )));
                continue;
            };
            match fs::read(dir.join(output_path)) {
                Ok(bytes) => {
                    let actual = sha256_hex(&bytes);
                    if actual != attempt.output_hash {
                        findings.push(error(format!(
                            "attempt {} output hash mismatch for {}",
                            attempt.attempt, record.agent_key
                        )));
                    }
                }
                Err(_) => findings.push(error(format!(
                    "attempt {} output file missing for {}",
                    attempt.attempt, record.agent_key
                ))),
            }
        }
    }

    if let Ok(state) = store.load_state(session_id)
        && state.completed_agent_count != agents.len()
    {
        findings.push(error(format!(
            "state completed_agent_count={} but {} agent records found",
            state.completed_agent_count,
            agents.len()
        )));
    }

    let valid = !findings.iter().any(|f| f.severity == "error");
    let report = BundleVerificationReport {
        session_id: session_id.to_string(),
        valid,
        checked_at: Utc::now().to_rfc3339(),
        manifest_present,
        state_present,
        audit_events,
        agent_records: agents.len(),
        findings,
    };

    if write_report {
        let path = dir.join("verification").join("report.json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(&report)?)?;
    }

    Ok(report)
}

fn error(message: impl Into<String>) -> VerificationFinding {
    VerificationFinding {
        severity: "error".into(),
        message: message.into(),
    }
}

fn warn(message: impl Into<String>) -> VerificationFinding {
    VerificationFinding {
        severity: "warning".into(),
        message: message.into(),
    }
}
