#![allow(dead_code)]
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const EXTERNAL_PRD_ENV: &str = "ARCHON_R2A_EXTERNAL_PRD";
pub const EXTERNAL_TASK_ROOT_ENV: &str = "ARCHON_R2A_EXTERNAL_TASK_ROOT";
pub const PROTECTED_ROOT_ENV: &str = "ARCHON_R2A_PROTECTED_ROOT";
pub const EVIDENCE_ROOT_ENV: &str = "ARCHON_R2A_EVIDENCE_ROOT";
pub const SYNTHETIC_CLEARANCE_ENV: &str = "ARCHON_R2A_SYNTHETIC_CLEARANCE";
/// Set to `1` to run the external proof on a binary newer than the one the
/// synthetic clearance was minted on. Never implied: an engine fix must be a
/// conscious decision to skip proof 1, and the evidence records the mismatch.
pub const PRIOR_CLEARANCE_ENV: &str = "ARCHON_R2A_ACCEPT_PRIOR_CLEARANCE";

pub fn prior_clearance_accepted() -> bool {
    std::env::var(PRIOR_CLEARANCE_ENV).is_ok_and(|value| value.trim() == "1")
}
pub const PROOF_PROMPT_CANARY: &str = "SYNTHETIC-PROMPT-CANARY-7F3A91C2";
pub const PROOF_CANDIDATE_CANARY: &str = "SYNTHETIC-CANDIDATE-CANARY-4D8E62B1";
pub const PROOF_ENV_CANARY_NAME: &str = "ARCHON_PROOF_ENV_CANARY";
pub const PROOF_ENV_CANARY_VALUE: &str = "SYNTHETIC-ENV-CANARY-9C1B75E4";
pub const PROOF_SECRET_CANARY_NAME: &str = "ARCHON_PROOF_SECRET_CANARY";
pub const PROOF_SECRET_CANARY_VALUE: &str = "SYNTHETIC-SECRET-CANARY-2A6F80D3";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub source_revision: String,
    pub binary_sha256: String,
    pub binary_revision: String,
    pub script_digest: String,
    pub catalog_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityProbeReceipt {
    pub schema_version: u32,
    pub test_filter: String,
    pub runtime_identity_digest: String,
    pub exit_code: i32,
    pub output_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndependentReviewReceipt {
    pub schema_version: u32,
    pub reviewer: String,
    pub evidence_manifest_digest: String,
    pub runtime_identity_digest: String,
    pub artifact_path: String,
    pub artifact_sha256: String,
    pub approved: bool,
    pub unresolved_critical: u64,
    pub unresolved_important: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticClearance {
    pub schema_version: u32,
    pub identity: RuntimeIdentity,
    pub fixture_digest: String,
    pub evidence_manifest_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedEntry {
    pub relative_path: String,
    pub kind: String,
    pub mode: u32,
    pub byte_len: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedSnapshot {
    pub root: String,
    pub entries: Vec<ProtectedEntry>,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceEntry {
    pub relative_path: String,
    pub byte_len: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceManifest {
    pub schema_version: u32,
    pub package: String,
    pub identity: RuntimeIdentity,
    pub entries: Vec<EvidenceEntry>,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofPreflightFacts {
    pub runtime_exists: bool,
    pub runtime_is_file: bool,
    pub target_is_fresh: bool,
    pub observe_mode: bool,
    pub active_work: Vec<String>,
    pub idle_tui_count: usize,
    pub expected_idle_tui_count: usize,
    pub deployed_binaries_match: bool,
    pub protected_snapshot_valid: bool,
}

pub fn evaluate_preflight(facts: &ProofPreflightFacts) -> Result<(), String> {
    if !facts.active_work.is_empty() {
        return Err(format!(
            "proof preflight found active Archon/compiler work: {}",
            facts.active_work.join(" | ")
        ));
    }
    if facts.idle_tui_count != facts.expected_idle_tui_count {
        return Err(format!(
            "proof preflight expected {} idle Archon TUI process(es), found {}",
            facts.expected_idle_tui_count, facts.idle_tui_count
        ));
    }
    if !facts.runtime_exists || !facts.runtime_is_file {
        return Err("proof runtime path is missing or is not a regular file".into());
    }
    if !facts.target_is_fresh {
        return Err("proof target is not fresh for the requested package".into());
    }
    if !facts.observe_mode {
        return Err("proof requires workflow.gate_mode = observe".into());
    }
    if !facts.deployed_binaries_match {
        return Err("deployed Archon binaries differ".into());
    }
    if !facts.protected_snapshot_valid {
        return Err("protected tree snapshot preflight failed".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofAction {
    Decompose { prd: PathBuf, tasks: PathBuf },
    Status { run_id: String },
    Pause { run_id: String },
    Resume { run_id: String },
    ImplementSynthetic { tasks: PathBuf },
}

impl ProofAction {
    pub fn args(&self) -> Vec<String> {
        match self {
            ProofAction::Decompose { prd, tasks } => vec![
                "workflow".into(),
                "decompose".into(),
                "--prd".into(),
                prd.display().to_string(),
                "--tasks".into(),
                tasks.display().to_string(),
                "--yes".into(),
            ],
            ProofAction::Status { run_id } => {
                vec!["workflow".into(), "status".into(), run_id.clone()]
            }
            ProofAction::Pause { run_id } => {
                vec!["workflow".into(), "pause".into(), run_id.clone()]
            }
            ProofAction::Resume { run_id } => vec![
                "workflow".into(),
                "resume".into(),
                "--live".into(),
                "--yes".into(),
                run_id.clone(),
            ],
            ProofAction::ImplementSynthetic { tasks } => vec![
                "workflow".into(),
                "run".into(),
                "--live".into(),
                "--yes".into(),
                format!(
                    "Implement every canonical task in {} through the default v3 workflow and verify each declared focused test.",
                    tasks.display()
                ),
            ],
        }
    }
}

pub fn require_external_action(action: &ProofAction) -> Result<(), String> {
    match action {
        ProofAction::Decompose { .. }
        | ProofAction::Status { .. }
        | ProofAction::Pause { .. }
        | ProofAction::Resume { .. } => Ok(()),
        ProofAction::ImplementSynthetic { .. } => {
            Err("external proof is decomposition-only; implementation action refused".into())
        }
    }
}

pub fn rust_function_body<'a>(source: &'a str, signature: &str) -> Result<&'a str, String> {
    let signature_start = source
        .rfind(signature)
        .ok_or_else(|| format!("Rust function signature not found: {signature}"))?;
    let open = source[signature_start..]
        .find('{')
        .map(|offset| signature_start + offset)
        .ok_or_else(|| format!("Rust function has no body: {signature}"))?;
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for index in open..bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Ok(&source[open + 1..index]);
            }
        }
    }
    Err(format!("Rust function body is unclosed: {signature}"))
}

pub fn parse_started_run_id(output: &str) -> Result<String, String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Fixed decomposition started: "))
        .filter(|id| id.starts_with("wf-") && id.len() > 3)
        .map(str::to_string)
        .ok_or_else(|| "fixed decomposition output carried no persisted run id".into())
}

pub fn synthetic_fixture_digest() -> String {
    sha256(include_bytes!("fixtures/decomposition-synthetic/prd.md"))
}

pub fn read_clearance(path: &Path) -> Result<SyntheticClearance, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "synthetic clearance {} is unavailable: {error}",
            path.display()
        )
    })?;
    let clearance: SyntheticClearance = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "synthetic clearance {} is malformed: {error}",
            path.display()
        )
    })?;
    if clearance.fixture_digest != synthetic_fixture_digest() {
        return Err("synthetic clearance fixture digest differs from committed fixture".into());
    }
    if clearance.schema_version != 1 {
        return Err(format!(
            "synthetic clearance schema_version must be 1, found {}",
            clearance.schema_version
        ));
    }
    Ok(clearance)
}

pub fn require_clearance_identity(
    clearance: &SyntheticClearance,
    current: &RuntimeIdentity,
) -> Result<(), String> {
    if &clearance.identity != current {
        return Err("synthetic clearance identity differs from current deployed runtime".into());
    }
    Ok(())
}

pub fn common_existing_ancestor(left: &Path, right: &Path) -> Result<PathBuf, String> {
    let left = left
        .canonicalize()
        .map_err(|error| format!("canonicalizing {}: {error}", left.display()))?;
    let right = right
        .canonicalize()
        .map_err(|error| format!("canonicalizing {}: {error}", right.display()))?;
    let mut candidate = if left.is_dir() {
        left
    } else {
        left.parent()
            .ok_or_else(|| "left path has no parent".to_string())?
            .to_path_buf()
    };
    let right = if right.is_dir() {
        right
    } else {
        right
            .parent()
            .ok_or_else(|| "right path has no parent".to_string())?
            .to_path_buf()
    };
    while !right.starts_with(&candidate) {
        candidate = candidate
            .parent()
            .ok_or_else(|| "paths have no common existing ancestor".to_string())?
            .to_path_buf();
    }
    Ok(candidate)
}

pub fn build_evidence_manifest(
    package: &str,
    identity: RuntimeIdentity,
    root: &Path,
) -> Result<EvidenceManifest, String> {
    build_evidence_manifest_excluding(
        package,
        identity,
        root,
        &["manifest.json", "clearance.json"],
    )
}

pub fn build_pre_review_manifest(
    package: &str,
    identity: RuntimeIdentity,
    root: &Path,
    review_artifact: &str,
) -> Result<EvidenceManifest, String> {
    build_evidence_manifest_excluding(
        package,
        identity,
        root,
        &[
            "manifest.json",
            "clearance.json",
            "review.json",
            review_artifact,
        ],
    )
}

fn build_evidence_manifest_excluding(
    package: &str,
    identity: RuntimeIdentity,
    root: &Path,
    excluded: &[&str],
) -> Result<EvidenceManifest, String> {
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("evidence root {} is unavailable: {error}", root.display()))?;
    let mut paths = Vec::new();
    collect_regular_files(&canonical, &canonical, &mut paths)?;
    paths.sort();
    let entries = paths
        .into_iter()
        .filter(|path| !excluded.iter().any(|name| path == Path::new(name)))
        .map(|relative| {
            let bytes = read_regular_nofollow(&canonical.join(&relative))?;
            Ok(EvidenceEntry {
                relative_path: slash_path(&relative),
                byte_len: bytes.len() as u64,
                sha256: sha256(&bytes),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let digest = digest_json(&(package, &identity, &entries))?;
    Ok(EvidenceManifest {
        schema_version: 1,
        package: package.to_string(),
        identity,
        entries,
        digest,
    })
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("creating {}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| format!("writing {}: {error}", path.display()))
}

#[path = "support/workflow_decomposition_proof_files.rs"]
mod files;
pub(crate) use files::{collect_regular_files, read_regular_nofollow};
pub use files::{require_unchanged, snapshot_tree};

#[path = "support/workflow_decomposition_proof_evidence.rs"]
mod evidence;
#[path = "support/workflow_decomposition_proof_log.rs"]
mod log_lines;
pub use evidence::*;
#[path = "support/workflow_decomposition_proof_runtime.rs"]
mod runtime;
pub use runtime::*;
#[path = "support/workflow_decomposition_proof_config.rs"]
mod proof_config;
pub use proof_config::*;
#[path = "support/workflow_decomposition_proof_progress.rs"]
mod progress;
pub use progress::*;

pub fn identity_digest(identity: &RuntimeIdentity) -> Result<String, String> {
    digest_json(identity)
}

pub fn bytes_sha256(bytes: &[u8]) -> String {
    sha256(bytes)
}

fn digest_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| error.to_string())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
#[path = "support/workflow_decomposition_proof_support_tests.rs"]
mod tests;
