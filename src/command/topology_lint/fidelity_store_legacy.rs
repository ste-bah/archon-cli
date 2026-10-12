//! The one-time migration of schema-4 verdict records (#434c).
//!
//! Schema 4 keyed a record on the whole build, so every deploy re-asked
//! every obligation of a long-lived run, and the critic flips verdicts on
//! byte-identical text. Schema 5 keys on a digest of the critic's own logic
//! (`ARCHON_FIDELITY_LOGIC_DIGEST`) instead. A schema-4 record is still a
//! verdict on the same question when, and only when, all of these hold:
//!
//! - this build's critic logic is the logic every schema-4 record was
//!   written by ([`LEGACY_CRITIC_LOGIC`]). Once any hashed critic file
//!   changes, the per-task rendering, the obligation JSON, the re-ask
//!   wording or the reply checks may differ from what the empty-question
//!   request identity shows, so no schema-4 record is read at all;
//! - its obligation, skeleton digest, model, provider and request identity
//!   are the question's;
//! - every entry of it judged over the question's tasks and texts passes
//!   the current fresh-reply check ([`verdict_holds`]) again.
//!
//! One obligation can have several schema-4 records, one per deploy, over the
//! same texts: those are exactly the flips of #434. Their entries are merged
//! by the rule a schema-5 record uses — the latest true and the latest false
//! verdict — so a real disagreement becomes a recorded contradiction, never a
//! verdict chosen by directory order. A schema-5 record under the question's
//! key supersedes every schema-4 record: legacy records are read only for a
//! question that has none, and what a migration returns is always what the
//! record it wrote (or found written under the lock) says.
//!
//! The scan never fails a gate. A directory that cannot be read, an entry or
//! record file that cannot be read, or more than [`MAX_LEGACY_RECORDS`] record
//! files stop it with no candidates at all — the critic is asked, as when no
//! record existed — so a partial scan can never hide one side of a conflict.
//! A file that does not parse as a schema-4 record (a schema-5 record, a
//! foreign or torn file) or whose key does not verify is not a candidate.

use std::path::{Path, PathBuf};

use archon_workflow::fidelity_audit::{ClaimedObligation, verdict_holds};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{Entry, Question, Standing, StoreIdentity, VerdictStore, standing};

pub(super) const LEGACY_STORE_SCHEMA: u32 = 4;

/// The critic-logic digest (`fidelity_logic_digest::digest`, as `build.rs`
/// computes `ARCHON_FIDELITY_LOGIC_DIGEST`) of the four files it hashed when
/// this migration was written — `fidelity_audit.rs`, `fidelity_audit_check.rs`,
/// `fidelity_audit_reply.rs` and `fidelity_critic.rs` — at commit bc8cf2693,
/// which introduced schema 4. No later commit up to c73736abf (the last one
/// that wrote schema 4) changed any of them, so every schema-4 record was
/// written by this logic. The value is frozen: it never follows a later
/// change, which then simply ends the migration. The ignored test
/// `legacy_critic_logic_is_the_schema_four_sources_digest` recomputes it
/// from git history.
pub(in crate::command::topology_lint) const LEGACY_CRITIC_LOGIC: &str =
    "631274368c9492c557bedefacf3d203ce7072d51c4cdd682aff072608c741c7a";

/// Record files one gate reads at most while looking for schema-4 records.
const MAX_LEGACY_RECORDS: usize = 10_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyIdentity {
    binary: String,
    model: String,
    provider: Option<String>,
    request: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::command::topology_lint) struct LegacyRecord {
    schema: u32,
    key: String,
    obligation: ClaimedObligation,
    skeleton: String,
    identity: LegacyIdentity,
    entries: Vec<Entry>,
}

impl LegacyRecord {
    fn key(&self) -> String {
        let material = serde_json::json!([
            LEGACY_STORE_SCHEMA,
            self.obligation,
            self.skeleton,
            {
                "binary": self.identity.binary,
                "model": self.identity.model,
                "provider": self.identity.provider,
                "request": self.identity.request,
            }
        ]);
        hex::encode(Sha256::digest(material.to_string().as_bytes()))
    }

    fn asks(&self, question: Question<'_>, identity: &StoreIdentity) -> bool {
        self.obligation == *question.obligation
            && self.skeleton == question.skeleton
            && self.identity.model == identity.model
            && self.identity.provider == identity.provider
            && self.identity.request == identity.request
    }
}

impl VerdictStore {
    /// The schema-4 candidates, read once per gate; none unless this build's
    /// critic logic is [`LEGACY_CRITIC_LOGIC`] and the request is known.
    pub(in crate::command::topology_lint) fn legacy_records(&self) -> Vec<LegacyRecord> {
        self.legacy_records_within(MAX_LEGACY_RECORDS)
    }

    pub(super) fn legacy_records_within(&self, limit: usize) -> Vec<LegacyRecord> {
        if !self.migrates() {
            return Vec::new();
        }
        scan(&self.dir, limit).unwrap_or_else(|reason| {
            tracing::warn!(
                "fidelity store {}: schema-4 records not migrated, every obligation without a \
                 schema-5 record is asked again: {reason}",
                self.dir.display()
            );
            Vec::new()
        })
    }

    /// Schema-4 verdicts answer this store's questions only under the critic
    /// logic they were written by, and only for a known request.
    fn migrates(&self) -> bool {
        self.identity.critic_logic == LEGACY_CRITIC_LOGIC && self.identity.request.is_some()
    }

    /// What stands for `question`: its schema-5 record when it has one;
    /// otherwise the merged schema-4 entries for it, written as its schema-5
    /// record first. `None` when it must be asked.
    pub(in crate::command::topology_lint) fn load_or_migrate(
        &self,
        question: Question<'_>,
        records: &[LegacyRecord],
    ) -> Option<Standing> {
        if records.is_empty() || !self.migrates() || self.read(question).is_some() {
            return self.load(question);
        }
        let entries = merged(records, question, &self.identity)?;
        let key = self.key(question);
        let promoted = self.locked(&key, || {
            // Another gate may have saved or migrated it meanwhile; that
            // record is newer evidence and is kept.
            if self.read(question).is_none() {
                self.write(&self.record(question, key.clone(), entries))?;
            }
            Ok(())
        });
        if let Err(error) = promoted {
            // As before migration existed: a record that cannot be had is
            // asked again, and a store that cannot be written fails its save.
            tracing::warn!("fidelity schema-4 record not migrated: {error:#}");
            return None;
        }
        self.load(question)
    }
}

/// Every verified schema-4 record in `dir`, in path order.
fn scan(dir: &Path, limit: usize) -> Result<Vec<LegacyRecord>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("reading the directory: {error}")),
    };
    let mut found: Vec<(PathBuf, LegacyRecord)> = Vec::new();
    let mut scanned = 0usize;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("reading a directory entry: {error}"))?
            .path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        scanned += 1;
        if scanned > limit {
            return Err(format!("more than {limit} record files"));
        }
        let bytes =
            std::fs::read(&path).map_err(|error| format!("reading {}: {error}", path.display()))?;
        let Ok(record) = serde_json::from_slice::<LegacyRecord>(&bytes) else {
            continue;
        };
        if record.schema == LEGACY_STORE_SCHEMA
            && record.key == record.key()
            && path
                .file_stem()
                .is_some_and(|stem| stem == std::ffi::OsStr::new(&record.key))
        {
            found.push((path, record));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found.into_iter().map(|(_, record)| record).collect())
}

/// The latest true and the latest false entry, over all `records` that ask
/// `question`, judged over exactly its tasks and texts; `None` when there is
/// none, or when any such entry is foreign, undated or fails the current
/// fresh-reply check.
fn merged(
    records: &[LegacyRecord],
    question: Question<'_>,
    identity: &StoreIdentity,
) -> Option<Vec<Entry>> {
    let mut dated = Vec::new();
    for record in records
        .iter()
        .filter(|record| record.asks(question, identity))
    {
        for entry in &record.entries {
            if entry.verdict.obligation_id != question.obligation.id {
                return None;
            }
            if entry.tasks != *question.texts {
                continue;
            }
            if !verdict_holds(&entry.verdict, question.tasks) {
                return None;
            }
            let at = chrono::DateTime::parse_from_rfc3339(&entry.audited_at).ok()?;
            dated.push((at, entry));
        }
    }
    // Stable: equal times keep path order.
    dated.sort_by_key(|(at, _)| *at);
    let latest = |truth: bool| {
        dated
            .iter()
            .rev()
            .find(|(_, entry)| entry.verdict.necessarily_true == truth)
    };
    let mut kept: Vec<_> = [latest(false), latest(true)]
        .into_iter()
        .flatten()
        .collect();
    kept.sort_by_key(|(at, _)| *at);
    let kept: Vec<Entry> = kept
        .into_iter()
        .map(|(_, entry)| (*entry).clone())
        .collect();
    standing(&kept, question)?;
    Some(kept)
}
