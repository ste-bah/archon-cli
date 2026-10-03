//! The fidelity audit's saved verdicts: one record per call batch, so a set
//! gate the host stops, or a later gate over the same texts, reuses every
//! batch already answered (Issue 259).
//!
//! # The key
//!
//! A record is found under a digest of everything that decides the answer:
//!
//! - the batch's question, [`fidelity_cluster_digest`]: its obligation ids
//!   and texts, its task ids and full texts, and the frozen skeleton
//!   section, all recomputed from the files on every run, so an edited
//!   input never meets an old verdict;
//! - the binary revision (`ARCHON_GIT_HASH`), which fixes the prompt, the
//!   reply parser and the critic's settings, as the freeze keys its probe
//!   verdicts (Issue 255);
//! - the critic's identity under the provider environment the call runs
//!   in: the model the critic alias resolves to, the provider id, and the
//!   client's request identity (a digest of its endpoint and output
//!   ceiling, never a secret: `WorkflowLlmClient::request_identity`).
//!
//! # Fail closed
//!
//! A record is trusted only when its schema, key, digest and identity all
//! match and its verdicts pass, again, the validation a fresh reply passes
//! ([`parse_fidelity_response`]): exactly the batch's ids, each verdict's
//! provenance inside the batch's tasks. Anything else (unreadable, torn,
//! foreign, forged) is no record, and the batch is asked again. A record is
//! written to a temporary file and renamed into place, so a process killed
//! mid-write leaves no partial record under a real key.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, FidelityVerdict, parse_fidelity_response,
};
use archon_workflow::llm_client_port::WorkflowLlmClient;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::fidelity_critic::CRITIC_MODEL_ALIAS;

/// Records of another layout are never read as this one.
const STORE_SCHEMA: u32 = 2;

/// Where the saved verdicts live: under the project, outside the host
/// command's call staging directory, which the executor clears before every
/// attempt.
pub(super) fn store_dir(cwd: &Path) -> PathBuf {
    cwd.join(".archon").join("lint-cache").join("fidelity")
}

/// What, besides the question, decides a critic's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StoreIdentity {
    binary: String,
    model: String,
    provider: Option<String>,
    request: Option<String>,
}

impl StoreIdentity {
    /// This binary asking `client`'s critic.
    pub(super) fn of(client: &dyn WorkflowLlmClient) -> Self {
        Self {
            binary: env!("ARCHON_GIT_HASH").to_string(),
            model: client.resolve_model_alias(CRITIC_MODEL_ALIAS),
            provider: client.provider_id(),
            request: client.request_identity(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_binary(mut self, binary: &str) -> Self {
        self.binary = binary.to_string();
        self
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    key: String,
    digest: String,
    identity: StoreIdentity,
    audited_at: String,
    verdicts: Vec<FidelityVerdict>,
}

/// One record per call batch, under one identity.
pub(super) struct VerdictStore {
    dir: PathBuf,
    identity: StoreIdentity,
}

impl VerdictStore {
    pub(super) fn new(dir: PathBuf, identity: StoreIdentity) -> Self {
        Self { dir, identity }
    }

    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    fn key(&self, digest: &str) -> String {
        let material = serde_json::json!([STORE_SCHEMA, digest, self.identity]);
        hex::encode(Sha256::digest(material.to_string().as_bytes()))
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// The verdicts saved for the batch `digest` asks about `obligations`
    /// over `tasks`, re-validated; `None` for anything less than a record
    /// this store would write now.
    pub(super) fn load(
        &self,
        digest: &str,
        obligations: &[ClaimedObligation],
        tasks: &[ClaimingTask],
    ) -> Option<Vec<FidelityVerdict>> {
        let key = self.key(digest);
        let record: Record = serde_json::from_slice(&std::fs::read(self.path(&key)).ok()?).ok()?;
        if record.schema != STORE_SCHEMA
            || record.key != key
            || record.digest != digest
            || record.identity != self.identity
        {
            return None;
        }
        let document = serde_json::json!({ "verdicts": record.verdicts }).to_string();
        parse_fidelity_response(&document, obligations, tasks).ok()
    }

    /// Save `verdicts` for the batch `digest`, atomically.
    pub(super) fn save(&self, digest: &str, verdicts: &[FidelityVerdict]) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating fidelity store {}", self.dir.display()))?;
        let key = self.key(digest);
        let record = Record {
            schema: STORE_SCHEMA,
            key: key.clone(),
            digest: digest.to_string(),
            identity: self.identity.clone(),
            audited_at: chrono::Utc::now().to_rfc3339(),
            verdicts: verdicts.to_vec(),
        };
        let path = self.path(&key);
        let staging = self.dir.join(format!(".{key}.{}.tmp", std::process::id()));
        let written = serde_json::to_vec_pretty(&record)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(std::fs::write(&staging, bytes)?))
            .and_then(|()| Ok(std::fs::rename(&staging, &path)?));
        if written.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
        written.with_context(|| format!("writing fidelity record {}", path.display()))
    }
}
