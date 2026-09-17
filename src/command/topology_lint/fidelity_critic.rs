//! The fidelity critic call and its verdict cache, split from `fidelity.rs`
//! so the audit file stays under its size budget. Nothing here decides what
//! is asked or what a verdict means: `fidelity.rs` builds the clusters and
//! reads the answers; this file asks, bounds, and remembers.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, FidelityVerdict, SkeletonSummary, fidelity_prompt,
    parse_fidelity_response,
};
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};

/// The alias the audit asks for. The critic reads whole task files and is
/// asked to find the sentence that lets a claim go hollow; that is the
/// strongest tier the provider offers, whatever it resolves to.
pub(super) const CRITIC_MODEL_ALIAS: &str = "opus";
/// One re-ask on a malformed reply, then the failure is operational. A
/// formatting slip often corrects on a second pass; a third pass is spend.
pub(super) const FIDELITY_ATTEMPTS: usize = 2;
const FIDELITY_CALL_TIMEOUT_SECS: u64 =
    crate::command::workflow_task_set::judge::JUDGE_TIMEOUT_SECS;

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedCluster {
    digest: String,
    model: String,
    provider: Option<String>,
    audited_at: String,
    verdicts: Vec<FidelityVerdict>,
}

/// Ask once, re-ask once on a malformed reply, and keep every rejected reply
/// under `rejected/` in the cache directory — the operator who is told "no
/// usable verdict" needs to see what the critic actually said.
pub(super) async fn ask(
    client: &dyn WorkflowLlmClient,
    cache: &Path,
    digest: &str,
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
    skeleton: &SkeletonSummary,
) -> Result<Vec<FidelityVerdict>> {
    let prompt = fidelity_prompt(obligations, tasks, skeleton);
    let mut last = String::from("never asked");
    for attempt in 1..=FIDELITY_ATTEMPTS {
        let outcome = tokio::time::timeout(
            Duration::from_secs(FIDELITY_CALL_TIMEOUT_SECS),
            client.send_message_with_temperature(
                vec![serde_json::json!({ "role": "user", "content": prompt.clone() })],
                Vec::new(),
                Vec::new(),
                CRITIC_MODEL_ALIAS,
                0.0,
            ),
        )
        .await
        .map_err(|_| anyhow!("fidelity critic timed out after {FIDELITY_CALL_TIMEOUT_SECS}s"))?
        .map_err(anyhow::Error::new)?;
        // A truncated reply is not re-asked: the budget that cut it off has
        // not changed, and partial JSON is never repaired into a verdict.
        require_complete(&outcome)?;
        let document = crate::command::workflow_freeze_candidate::candidate_document(
            outcome.content.trim().as_bytes(),
        );
        let document = String::from_utf8_lossy(&document);
        match parse_fidelity_response(&document, obligations, tasks) {
            Ok(verdicts) => return Ok(verdicts),
            Err(error) => {
                let rejected = cache.join("rejected");
                let path = rejected.join(format!("{digest}-attempt-{attempt}.txt"));
                let kept = std::fs::create_dir_all(&rejected)
                    .and_then(|()| std::fs::write(&path, &outcome.content))
                    .map(|()| path.display().to_string())
                    .unwrap_or_else(|error| format!("not kept: {error}"));
                last = format!("{error} (reply kept at {kept})");
            }
        }
    }
    Err(anyhow!(
        "fidelity critic returned no usable verdict for {:?} after {FIDELITY_ATTEMPTS} attempts: {last}",
        obligations
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>()
    ))
}

fn require_complete(outcome: &WorkflowAgentOutcome) -> Result<()> {
    match outcome.stop_reason.as_deref() {
        Some("end_turn" | "stop" | "completed") => Ok(()),
        Some(reason) => Err(anyhow!(
            "fidelity critic reply ended with stop reason '{reason}'; a truncated verdict is never parsed"
        )),
        None => Err(anyhow!(
            "fidelity critic returned no finish reason; refusing to parse a possibly truncated verdict"
        )),
    }
}

pub(super) fn cache_dir(cwd: &Path) -> PathBuf {
    cwd.join(".archon").join("lint-cache").join("fidelity")
}

pub(super) fn read_cached(path: &Path, digest: &str) -> Option<Vec<FidelityVerdict>> {
    let cached: CachedCluster = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (cached.digest == digest).then_some(cached.verdicts)
}

pub(super) fn write_cached(
    path: &Path,
    client: &dyn WorkflowLlmClient,
    digest: &str,
    verdicts: &[FidelityVerdict],
) -> Result<()> {
    let parent = path.parent().expect("cache path has a parent");
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating fidelity cache {}", parent.display()))?;
    let cached = CachedCluster {
        digest: digest.to_string(),
        model: client.resolve_model_alias(CRITIC_MODEL_ALIAS),
        provider: client.provider_id(),
        audited_at: chrono::Utc::now().to_rfc3339(),
        verdicts: verdicts.to_vec(),
    };
    std::fs::write(path, serde_json::to_vec_pretty(&cached)?)
        .with_context(|| format!("writing fidelity cache {}", path.display()))
}
