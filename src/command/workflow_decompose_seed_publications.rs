//! When each published acceptance entry got the bytes it holds now, read
//! from durable evidence the publish paths write: the acceptance lock (every
//! whole-set freeze and every per-check republish — resume heal, operator
//! re-author, launch repair, in-round repair — rewrites it with the contract
//! digest it published and its gate stamp time), this run's freeze
//! publication receipts, and the resolved-journal rows a resume heal writes.
//!
//! A seeded author reply may overlay its published entry only when this
//! evidence shows the entry already held its current bytes when the reply
//! was authored. Without such evidence the published entry wins: a repair
//! published after the reply is never replaced by the reply's older bytes.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use archon_workflow::WorkflowV2CallRecord;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, AcceptanceCriterion,
    AcceptanceLock,
};
use chrono::{DateTime, FixedOffset};

use crate::command::workflow_decompose_remediation as remediation;

type At = DateTime<FixedOffset>;

/// One observed publication of one entry: when, the digest recorded for it,
/// and whether that digest uses this build's entry identity.
struct Seen {
    at: At,
    digest: String,
    versioned: bool,
}

pub(super) struct Publications {
    seen: BTreeMap<String, Vec<Seen>>,
}

impl Publications {
    /// The evidence for the published `contract` (whose bytes hash to
    /// `contract_digest`) under `task_root`, and this run's own records.
    pub(super) fn read(
        task_root: &Path,
        run_root: &Path,
        (contract, contract_digest): (&AcceptanceContract, &str),
        records: &[WorkflowV2CallRecord],
    ) -> Result<Self> {
        let mut publications = Self {
            seen: BTreeMap::new(),
        };
        let current = || contract.acceptance.iter().chain(&contract.supplementary);
        let lock_path = task_root.join(ACCEPTANCE_LOCK_FILE);
        match std::fs::read(&lock_path) {
            Ok(bytes) => {
                let lock: AcceptanceLock = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing {}", lock_path.display()))?;
                if let (true, Ok(at)) = (
                    lock.digest == contract_digest,
                    DateTime::parse_from_rfc3339(&lock.gate.evaluated_at),
                ) {
                    current().for_each(|entry| publications.saw(at, entry));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", lock_path.display()));
            }
        }
        for record in records {
            let Some((at, digest)) = contract_receipt(record) else {
                continue;
            };
            if digest == contract_digest {
                current().for_each(|entry| publications.saw(at, entry));
                continue;
            }
            // An earlier contract: its entries as this run submitted them.
            let stdin = record
                .call
                .options
                .host_command
                .as_ref()
                .and_then(|request| request.stdin.as_deref())
                .unwrap_or_default();
            let Ok(entries) =
                super::super::derive::record::candidate_entries(stdin, &record.call.id)
            else {
                continue;
            };
            for (_, entry, _) in entries {
                if let Some(entry) = authored_only(&entry) {
                    publications.saw(at, &entry);
                }
            }
        }
        for (id, at, digest, versioned) in remediation::resolved_publications(run_root)? {
            publications.seen.entry(id).or_default().push(Seen {
                at,
                digest,
                versioned,
            });
        }
        for seen in publications.seen.values_mut() {
            seen.sort_by_key(|seen| seen.at);
        }
        Ok(publications)
    }

    fn saw(&mut self, at: At, entry: &AcceptanceCriterion) {
        self.seen.entry(entry.id.clone()).or_default().push(Seen {
            at,
            digest: remediation::authored_entry_digest(entry),
            versioned: true,
        });
    }

    /// The earliest time the evidence shows `current` published and held
    /// since: the oldest of the newest-first observations that all name
    /// its bytes. `None` when no observation names them.
    pub(super) fn unchanged_since(&self, current: &AcceptanceCriterion) -> Option<At> {
        let mut since = None;
        for seen in self.seen.get(&current.id).into_iter().flatten().rev() {
            if !remediation::recorded_digest_names(&seen.digest, seen.versioned, current) {
                break;
            }
            since = Some(seen.at);
        }
        since
    }
}

/// The commit time and contract digest of a freeze record's publication
/// receipt, whichever gate logic judged it: publication is a fact.
fn contract_receipt(record: &WorkflowV2CallRecord) -> Option<(At, String)> {
    let request = record.call.options.host_command.as_ref()?;
    if request.command_id != "freeze-acceptance" {
        return None;
    }
    let receipt: archon_workflow::PublicationReceiptV1 =
        serde_json::from_value(record.result.data.get("publicationReceipt")?.clone()).ok()?;
    let at = DateTime::parse_from_rfc3339(&receipt.committed_at).ok()?;
    let entry = receipt.entries.iter().find(|entry| {
        Path::new(&entry.destination_path)
            .file_name()
            .is_some_and(|name| name == ACCEPTANCE_CONTRACT_FILE)
    })?;
    Some((at, entry.blake3.clone()))
}

/// A submitted candidate entry's authored fields as a contract entry; its
/// judgment is host-owned and outside the authored identity.
fn authored_only(entry: &serde_json::Value) -> Option<AcceptanceCriterion> {
    let mut entry = entry.clone();
    let object = entry.as_object_mut()?;
    object.insert(
        "judgment".into(),
        serde_json::json!({"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}),
    );
    serde_json::from_value(entry).ok()
}

#[cfg(test)]
#[path = "workflow_decompose_seed_publications_tests.rs"]
pub(super) mod tests;
