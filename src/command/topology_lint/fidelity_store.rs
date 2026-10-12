//! The fidelity audit's saved verdicts: one record per obligation, so a set
//! gate the host stops, a later body round, or the set gate after the body
//! gates, reuses every verdict judged on exactly the texts it would be
//! asked about now (Issues 259, 434).
//!
//! # Why per obligation (#434)
//!
//! Records were per call batch: a batch held up to eight obligations of one
//! cluster, and any byte changed in any task of the cluster re-asked all of
//! them. The critic does not repeat its answers on the same question (live:
//! verdicts flipped on byte-identical lines between rounds), so every
//! needless re-ask was a fresh chance to flip. A record now belongs to one
//! obligation and holds the set of tasks it was judged over and a digest of
//! each task's text; a saved verdict, true or false, stands only while that
//! set is identical and every one of those texts is byte-identical. An
//! edited task re-asks only the obligations whose cluster holds it.
//!
//! What a verdict rests on is never narrowed below that. A true verdict's
//! `supporting_text` is verified evidence that makes the verdict auditable,
//! but it is not a reuse anchor: a qualifier beside the quote, the quote
//! moved under another heading, or a loophole added anywhere in any claimer
//! can each make the obligation false while the quote is still there.
//!
//! # Contradictions are recorded, never resolved silently
//!
//! A record keeps at most one true and one false verdict. A fresh answer
//! replaces the record's verdict of the same meaning; one of the other
//! meaning that still stands (same tasks, same texts) is kept beside it.
//! Both standing at once — two gates asked the same obligation side by
//! side over the same texts and got different answers — is a contradiction:
//! it is reported as one blocking finding quoting both, and neither verdict
//! is taken. Once either stops standing the obligation is asked again rather
//! than letting the survivor win unasked. No adjudication path exists for
//! fidelity verdicts besides the operator's waiver, so the finding
//! (waivable like any false verdict) is the record.
//!
//! # The key
//!
//! A record is found under a digest of the obligation's id and text, of the
//! frozen skeleton section (a re-frozen skeleton re-asks every obligation),
//! and of the critic's identity: a build fingerprint of source contents,
//! manifests, lockfile, compiler and build configuration (so a rebuilt
//! binary re-asks every obligation once); the model the critic alias
//! resolves to; the provider id; and the client's rendered request identity
//! for the question with no obligations and no tasks — effective system
//! blocks, the prompt's own wording and the frozen skeleton section, model,
//! output ceiling, sampling, tools and endpoint. Credentials and volatile
//! user/session attribution never enter it. Unknown envelopes are not
//! persisted or reused.
//!
//! # Fail closed
//!
//! A record is trusted only when its schema, key, obligation, skeleton and
//! identity all match, and a verdict in it is reused only when its tasks
//! and text digests match the current cluster and it passes, again, the
//! validation a fresh reply passes against the current texts
//! ([`verdict_holds`]). Anything else (unreadable, torn, foreign, forged) is
//! no record. A save holds an exclusive lock on `{key}.lock` while it reads
//! the record, merges the fresh verdict and renames the new record into
//! place, so two gates saving one obligation at once never lose a verdict
//! and a contradiction between them is always seen; the rename means a
//! process killed mid-write leaves no partial record under a real key.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, FidelityVerdict, verdict_holds,
};
use archon_workflow::llm_client_port::WorkflowLlmClient;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::fidelity_critic::CRITIC_MODEL_ALIAS;

/// Records of another layout are never read as this one. 4: per-obligation
/// records bound to their tasks' texts (#434).
const STORE_SCHEMA: u32 = 4;

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
            binary: env!("ARCHON_BUILD_FINGERPRINT").to_string(),
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

/// What stands for one obligation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Standing {
    Verdict(FidelityVerdict),
    /// A false and a true verdict that both still stand.
    Contradiction {
        refuted: FidelityVerdict,
        supported: FidelityVerdict,
    },
}

/// One obligation's question as it stands now: its text, the digest of the
/// skeleton section, its cluster's tasks, and each task's text digest.
#[derive(Clone, Copy)]
pub(super) struct Question<'a> {
    pub(super) obligation: &'a ClaimedObligation,
    pub(super) skeleton: &'a str,
    pub(super) tasks: &'a [ClaimingTask],
    pub(super) texts: &'a TaskTexts,
}

/// Every task of a cluster, by id, with the SHA-256 of its exact text.
pub(super) type TaskTexts = BTreeMap<String, String>;

/// `tasks` as a verdict over them is stored.
pub(super) fn task_texts(tasks: &[ClaimingTask]) -> TaskTexts {
    tasks
        .iter()
        .map(|task| {
            (
                task.task_id.clone(),
                hex::encode(Sha256::digest(task.text.as_bytes())),
            )
        })
        .collect()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    audited_at: String,
    /// The tasks it was judged over, each with its text digest.
    tasks: TaskTexts,
    verdict: FidelityVerdict,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    key: String,
    obligation: ClaimedObligation,
    skeleton: String,
    identity: StoreIdentity,
    entries: Vec<Entry>,
}

/// One record per obligation, under one identity.
pub(super) struct VerdictStore {
    dir: PathBuf,
    identity: StoreIdentity,
}

impl VerdictStore {
    pub(super) fn new(dir: PathBuf, identity: StoreIdentity) -> Self {
        Self { dir, identity }
    }

    /// The effective request envelope. Unknown envelopes cannot be reused.
    pub(super) fn for_request(&self, request: Option<String>) -> Self {
        let mut identity = self.identity.clone();
        identity.request = request;
        Self::new(self.dir.clone(), identity)
    }

    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    fn key(&self, question: Question<'_>) -> String {
        let material = serde_json::json!([
            STORE_SCHEMA,
            question.obligation,
            question.skeleton,
            self.identity
        ]);
        hex::encode(Sha256::digest(material.to_string().as_bytes()))
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// The entries saved for `question`'s obligation; `None` for anything
    /// less than a record this store would write now.
    fn read(&self, question: Question<'_>) -> Option<Vec<Entry>> {
        self.identity.request.as_ref()?;
        let key = self.key(question);
        let record: Record = serde_json::from_slice(&std::fs::read(self.path(&key)).ok()?).ok()?;
        (record.schema == STORE_SCHEMA
            && record.key == key
            && record.obligation == *question.obligation
            && record.skeleton == question.skeleton
            && record.identity == self.identity)
            .then_some(record.entries)
    }

    /// What stands for `question` from earlier answers; `None` when it must
    /// be asked.
    pub(super) fn load(&self, question: Question<'_>) -> Option<Standing> {
        standing(&self.read(question)?, question)
    }

    /// Save a fresh answer to `question`, atomically, and say what now
    /// stands: the answer, or a contradiction with a verdict of the other
    /// meaning that still stands. The flag is whether a record was written.
    pub(super) fn save(
        &self,
        question: Question<'_>,
        verdict: &FidelityVerdict,
    ) -> Result<(Standing, bool)> {
        if self.identity.request.is_none() {
            return Ok((Standing::Verdict(verdict.clone()), false));
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating fidelity store {}", self.dir.display()))?;
        let key = self.key(question);
        // Read, merge and rename under one exclusive lock: a verdict another
        // gate saved for this obligation a moment ago is merged, never lost.
        let lock_path = self.dir.join(format!("{key}.lock"));
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("opening fidelity lock {}", lock_path.display()))?;
        let mut lock = fd_lock::RwLock::new(lock_file);
        let _held = lock
            .write()
            .with_context(|| format!("locking fidelity record {}", lock_path.display()))?;
        let mut entries: Vec<Entry> = self
            .read(question)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| {
                entry.verdict.necessarily_true != verdict.necessarily_true && holds(entry, question)
            })
            .collect();
        entries.push(Entry {
            audited_at: chrono::Utc::now().to_rfc3339(),
            tasks: question.texts.clone(),
            verdict: verdict.clone(),
        });
        let standing = standing(&entries, question).ok_or_else(|| {
            anyhow!(
                "fidelity verdict for {} does not pass its own check against the texts it answered",
                question.obligation.id
            )
        })?;
        let record = Record {
            schema: STORE_SCHEMA,
            key: key.clone(),
            obligation: question.obligation.clone(),
            skeleton: question.skeleton.to_string(),
            identity: self.identity.clone(),
            entries,
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
        written.with_context(|| format!("writing fidelity record {}", path.display()))?;
        Ok((standing, true))
    }
}

/// Does `entry` still stand for `question`? It was judged over exactly
/// these tasks, every one with exactly this text, and its verdict passes the
/// fresh reply check against them. A true verdict's quote is checked as
/// evidence, never instead of the texts (#434).
fn holds(entry: &Entry, question: Question<'_>) -> bool {
    entry.verdict.obligation_id == question.obligation.id
        && entry.tasks == *question.texts
        && verdict_holds(&entry.verdict, question.tasks)
}

/// What a record's entries say now. One verdict stands while it holds. A
/// record holding both a true and a false verdict is a contradiction while
/// both hold; once either stops holding, the obligation is asked again.
fn standing(entries: &[Entry], question: Question<'_>) -> Option<Standing> {
    let latest = |truth: bool| {
        entries
            .iter()
            .rev()
            .find(|entry| entry.verdict.necessarily_true == truth)
    };
    match (latest(true), latest(false)) {
        (Some(supported), Some(refuted)) => (holds(supported, question)
            && holds(refuted, question))
        .then(|| Standing::Contradiction {
            refuted: refuted.verdict.clone(),
            supported: supported.verdict.clone(),
        }),
        (Some(only), None) | (None, Some(only)) => {
            holds(only, question).then(|| Standing::Verdict(only.verdict.clone()))
        }
        (None, None) => None,
    }
}

#[cfg(test)]
#[path = "fidelity_store_tests.rs"]
mod tests;
