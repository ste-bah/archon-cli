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
//! and of the critic's identity: the version of the fidelity question and
//! reply logic ([`CRITIC_LOGIC_VERSION`], guarded by a pinned digest of its
//! sources); the model the critic alias
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

// env! requires a literal, so derive the name and compiled value together.
macro_rules! build_environment {
    ($name:literal) => {
        pub(crate) const SOURCES_DIGEST_ENV: &str = $name;
        pub(crate) const SOURCES_DIGEST: &str = env!($name);
    };
}
build_environment!("ARCHON_FIDELITY_SOURCES_DIGEST");

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
pub(super) use logic_digest::CRITIC_LOGIC_VERSION;

/// Records of another layout are never read as this one. 4: per-obligation
/// records bound to their tasks' texts (#434). 5: the critic identity names a
/// digest of the critic's own logic instead of the whole build, so a rebuild
/// that leaves that logic alone reuses every verdict (#434c). 6: it names
/// [`CRITIC_LOGIC_VERSION`] instead of that digest, so an edit that leaves
/// a verdict's meaning alone reuses every verdict; schema-4 and schema-5
/// records are migrated once by [`legacy`] (#434d).
const STORE_SCHEMA: u32 = 7;

/// Where the saved verdicts live: under the project, outside the host
/// command's call staging directory, which the executor clears before every
/// attempt.
pub(super) fn store_dir(cwd: &Path) -> PathBuf {
    cwd.join(".archon").join("lint-cache").join("fidelity")
}

/// What, besides the question, decides a critic's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoreIdentity {
    critic_logic_version: u32,
    model: String,
    provider: Option<String>,
    request: Option<String>,
}

impl StoreIdentity {
    /// This binary asking `client`'s critic.
    pub(super) fn of(client: &dyn WorkflowLlmClient) -> Self {
        Self {
            critic_logic_version: CRITIC_LOGIC_VERSION,
            model: client.resolve_model_alias(CRITIC_MODEL_ALIAS),
            provider: client.provider_id(),
            request: client.request_identity(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_critic_logic_version(mut self, version: u32) -> Self {
        self.critic_logic_version = version;
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
    /// Whether this binary was built from the critic sources
    /// [`CRITIC_LOGIC_VERSION`] was reviewed against. When not, the version
    /// says nothing about the logic that would read or write a record, so
    /// no record is read, migrated or written (#434d).
    reviewed: bool,
}

impl VerdictStore {
    pub(super) fn new(dir: PathBuf, identity: StoreIdentity) -> Self {
        Self::built_from(dir, identity, SOURCES_DIGEST)
    }

    /// A store of a binary whose critic sources hash to `built`, as
    /// `build.rs` computes `ARCHON_FIDELITY_SOURCES_DIGEST`. Fails closed,
    /// never the gate: sources other than the pinned ones ask the critic
    /// afresh for every obligation and keep nothing.
    pub(super) fn built_from(dir: PathBuf, identity: StoreIdentity, built: &str) -> Self {
        let reviewed = built == logic_digest::SOURCES_DIGEST;
        if !reviewed {
            tracing::warn!(
                built,
                pinned = logic_digest::SOURCES_DIGEST,
                "fidelity critic sources of this build are not the pinned ones its critic logic \
                 version was reviewed against; no saved verdict is read, migrated or written, \
                 and every obligation is asked again"
            );
        }
        Self {
            dir,
            identity,
            reviewed,
        }
    }

    /// The effective request envelope. Unknown envelopes cannot be reused.
    pub(super) fn for_request(&self, request: Option<String>) -> Self {
        let mut identity = self.identity.clone();
        identity.request = request;
        Self {
            dir: self.dir.clone(),
            identity,
            reviewed: self.reviewed,
        }
    }

    /// Whether records may be read and written at all: a known envelope,
    /// under reviewed critic sources.
    fn keeps(&self) -> bool {
        self.reviewed && self.identity.request.is_some()
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
        if !self.keeps() {
            return None;
        }
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

    fn record(&self, question: Question<'_>, key: String, entries: Vec<Entry>) -> Record {
        Record {
            schema: STORE_SCHEMA,
            key,
            obligation: question.obligation.clone(),
            skeleton: question.skeleton.to_string(),
            identity: self.identity.clone(),
            entries,
        }
    }

    /// Run `under` holding the exclusive lock on `{key}.lock`.
    fn locked<T>(&self, key: &str, under: impl FnOnce() -> Result<T>) -> Result<T> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating fidelity store {}", self.dir.display()))?;
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
        under()
    }

    /// Stage `record` beside its path and rename it into place.
    fn write(&self, record: &Record) -> Result<()> {
        let path = self.path(&record.key);
        let staging = self
            .dir
            .join(format!(".{}.{}.tmp", record.key, std::process::id()));
        let written = serde_json::to_vec_pretty(record)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(std::fs::write(&staging, bytes)?))
            .and_then(|()| Ok(std::fs::rename(&staging, &path)?));
        if written.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
        written.with_context(|| format!("writing fidelity record {}", path.display()))
    }

    /// Save a fresh answer to `question`, atomically, and say what now
    /// stands: the answer, or a contradiction with a verdict of the other
    /// meaning that still stands. The flag is whether a record was written.
    pub(super) fn save(
        &self,
        question: Question<'_>,
        verdict: &FidelityVerdict,
    ) -> Result<(Standing, bool)> {
        if !self.keeps() {
            return Ok((Standing::Verdict(verdict.clone()), false));
        }
        let key = self.key(question);
        // Read, merge and rename under one exclusive lock. Keep separate task
        // text digests for the claimant cluster and whole-set question; a
        // later gate reuses only the entry whose exact task texts it asks.
        let standing = self.locked(&key, || {
            let mut entries: Vec<Entry> = self
                .read(question)
                .unwrap_or_default()
                .into_iter()
                .filter(|entry| {
                    entry.tasks != *question.texts
                        || (entry.verdict.necessarily_true != verdict.necessarily_true
                            && holds(entry, question))
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
            self.write(&self.record(question, key.clone(), entries))?;
            Ok(standing)
        })?;
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

/// The latest verdict of each meaning that still holds for these exact task
/// texts. A record with both is a contradiction for this question only.
fn standing(entries: &[Entry], question: Question<'_>) -> Option<Standing> {
    let latest = |truth: bool| {
        entries
            .iter()
            .rev()
            .find(|entry| entry.verdict.necessarily_true == truth && holds(entry, question))
    };
    match (latest(true), latest(false)) {
        (Some(supported), Some(refuted)) => Some(Standing::Contradiction {
            refuted: refuted.verdict.clone(),
            supported: supported.verdict.clone(),
        }),
        (Some(only), None) | (None, Some(only)) => Some(Standing::Verdict(only.verdict.clone())),
        (None, None) => None,
    }
}

#[path = "fidelity_store_legacy.rs"]
mod legacy;
#[cfg(test)]
pub(super) use legacy::{LEGACY_CRITIC_LOGIC, PREDECESSOR_VERSION};

#[path = "fidelity_logic_digest.rs"]
mod logic_digest;

#[cfg(test)]
#[path = "fidelity_store_tests.rs"]
mod tests;
