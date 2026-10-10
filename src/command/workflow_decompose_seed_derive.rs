//! The phase seed read from a run's own durable records (Issue 360).
//!
//! Nothing here decides acceptance. It reads, per author subject, the latest
//! candidate the old runtime produced, the gates that judged it and the
//! replies authored since, and checks every carried acceptance entry with this
//! build's freeze entry validator. The script turns that into its loop state
//! with its own rules (`workflow_decompose_v1_seed.js`). The acceptance
//! candidate it assembles goes to the gate again; a submission whose bytes a
//! recorded gate already judged keeps that recorded verdict while the gate's
//! logic version is unchanged (Issue 361, `workflow_host_command_logic`).
//!
//! Order is the script's own, never the wall clock: an author reply's place is
//! its call ordinal (`round * stride + 1` for acceptance), and a gate's place
//! is the reply round its candidate holds. Start times only order records the
//! ordinals cannot tell apart (gates of identical candidates, one round's
//! replies).
//!
//! The call-id families are the fixed decomposition script's own:
//! `acceptance-author-<entry id>-<n>` per acceptance entry, `<subject>-author-<n>`
//! for a whole artifact (the skeleton, each body), `pause-<subject>-<n>` for a
//! pause. No PRD, project or task name is known here; the host supplies only
//! the current requirement IDs and text needed to stamp supplementary checks.

use std::collections::BTreeMap;

use anyhow::Result;
use archon_workflow::{WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Status};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::seed_unmapped as unmapped;
#[path = "workflow_decompose_seed_host_owned.rs"]
mod host_owned;
use host_owned::HostOwned;
#[path = "workflow_decompose_seed_record.rs"]
mod record;

/// The acceptance gate. Its stdin is the candidate the script assembled.
const ACCEPTANCE_GATE: &str = "freeze-acceptance";
const ACCEPTANCE_PREFIX: &str = "acceptance-author-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateSeed {
    pub(crate) call_id: String,
    pub(crate) published: bool,
    /// The gate's policy findings, exactly as the script received them.
    pub(crate) findings: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplySeed {
    pub(crate) call_id: String,
    pub(crate) id: String,
    /// The JSON object the reply carried, as the script extracts it.
    pub(crate) text: String,
    /// The host-owned criterion in this author call's prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) criterion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SubjectSeed {
    /// Acceptance: entries authored one by one, assembled into the candidate.
    Entries {
        /// Every gate that judged a candidate of this subject, in order.
        gates: Vec<GateSeed>,
        /// The stdin of the last of them; absent when none judged one.
        candidate: Option<String>,
        /// The replies started after that gate, the latest per entry.
        replies: Vec<ReplySeed>,
        unreadable_replies: BTreeMap<String, String>,
        carried_entries: Vec<Value>,
        refuted_ids: Vec<String>,
        /// Carried entries this build's entry validator refuses, with why.
        invalid: BTreeMap<String, Vec<String>>,
        /// How many entries the seed carries.
        carried: usize,
    },
    /// A whole artifact (the skeleton, a body): its latest complete reply.
    Artifact {
        candidate: String,
        candidate_call: String,
        /// The last gate that judged exactly this candidate.
        gate: Option<GateSeed>,
    },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Derived {
    pub(crate) subjects: BTreeMap<String, SubjectSeed>,
    pub(crate) author_ordinals: BTreeMap<String, u64>,
    pub(crate) pause_ordinals: BTreeMap<String, u64>,
}

/// `(subject, ordinal)` of an author call id; acceptance entries keep their
/// entry id beside it.
fn author_call(id: &str) -> Option<(String, Option<String>, u64)> {
    let (head, ordinal) = id.rsplit_once('-')?;
    let ordinal = ordinal.parse::<u64>().ok()?;
    if let Some(entry) = head.strip_prefix(ACCEPTANCE_PREFIX)
        && !entry.is_empty()
    {
        return Some(("acceptance".into(), Some(entry.into()), ordinal));
    }
    let subject = head.strip_suffix("-author")?;
    (!subject.is_empty() && subject != "acceptance").then(|| (subject.into(), None, ordinal))
}

pub(crate) fn pause_ordinal(pause_id: &str) -> Option<(String, u64)> {
    let (head, ordinal) = pause_id.rsplit_once('-')?;
    let subject = head.strip_prefix("pause-").filter(|s| !s.is_empty())?;
    Some((subject.into(), ordinal.parse().ok()?))
}

fn started(record: &WorkflowV2CallRecord) -> Result<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(&record.started_at).map_err(|error| {
        unmapped(
            &format!("v2/results/{}.started_at", record.call.id),
            &error.to_string(),
        )
    })
}

/// A complete reply of an answered author call.
fn reply_content(record: &WorkflowV2CallRecord) -> Option<&str> {
    let data = &record.result.data;
    (record.status == WorkflowV2Status::Accepted
        && record.invalidated_by.is_none()
        && data["stopReason"] == "end_turn"
        && data["dry_run"] != true)
        .then(|| data["content"].as_str())
        .flatten()
        .filter(|content| !content.is_empty())
}

/// A gate outcome that judged or refused its candidate: an outage judged
/// nothing and is not evidence about it.
fn judged_gate(record: &WorkflowV2CallRecord) -> Option<(&str, &str, GateSeed)> {
    let request = record.call.options.host_command.as_ref()?;
    let data = &record.result.data;
    let envelope = data.get("gateEnvelope").filter(|v| v.is_object())?;
    // A gate envelope is evidence only for the logic that produced it. The
    // current catalog is authoritative: stale verdicts cannot seed repairs.
    let version = crate::command::workflow_host_command_logic::versions()
        .get(&request.command_id)
        .copied()?;
    let build_bound = crate::command::workflow_host_command_logic::digests()
        .get(&request.command_id)
        .is_some_and(|(_, bound)| *bound);
    if !crate::command::workflow_host_command_logic::outcome_logic_holds(
        data,
        version,
        false,
        build_bound.then_some(crate::command::workflow_host_command_logic::THIS_BUILD),
    ) {
        return None;
    }
    let judged = matches!(
        record.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::NeedsReview | WorkflowV2Status::Noop
    ) && record.invalidated_by.is_none()
        && envelope.get("operational_error").is_none_or(Value::is_null);
    judged.then(|| {
        let findings = envelope["policy_findings"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        (
            request.command_id.as_str(),
            request.stdin.as_deref().unwrap_or_default(),
            GateSeed {
                call_id: record.call.id.clone(),
                published: data["publicationReceipt"].is_object(),
                findings,
            },
        )
    })
}

/// The run's author calls and host commands, oldest first: the only records
/// a seed reads, and each must say when it started.
fn ordered(records: &[WorkflowV2CallRecord]) -> Result<Vec<&WorkflowV2CallRecord>> {
    let mut keyed = records
        .iter()
        .filter(|record| {
            matches!(
                record.call.method,
                WorkflowV2HostMethod::Agent | WorkflowV2HostMethod::HostCommand
            )
        })
        .map(|record| Ok((started(record)?, record)))
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_by(|(a, ra), (b, rb)| a.cmp(b).then_with(|| ra.call.id.cmp(&rb.call.id)));
    Ok(keyed.into_iter().map(|(_, record)| record).collect())
}

#[cfg(test)]
pub(crate) fn derive(
    records: &[WorkflowV2CallRecord],
    pause_ids: &[String],
    criteria: &BTreeMap<String, String>,
) -> Result<Derived> {
    derive_with_requirement_texts(records, pause_ids, criteria, &BTreeMap::new())
}

pub(crate) fn derive_with_requirement_texts(
    records: &[WorkflowV2CallRecord],
    pause_ids: &[String],
    criteria: &BTreeMap<String, String>,
    requirement_texts: &BTreeMap<String, String>,
) -> Result<Derived> {
    let records = ordered(records)?;
    let mut derived = Derived::default();
    for (subject, ordinal) in pause_ids.iter().filter_map(|id| pause_ordinal(id)) {
        let slot = derived.pause_ordinals.entry(subject).or_default();
        *slot = (*slot).max(ordinal);
    }
    let mut artifacts: BTreeMap<String, (u64, &WorkflowV2CallRecord)> = BTreeMap::new();
    for record in records
        .iter()
        .filter(|r| r.call.method == WorkflowV2HostMethod::Agent)
    {
        // Every recorded call holds its ordinal, failed and unfinished too.
        let Some((subject, entry, ordinal)) = author_call(&record.call.id) else {
            continue;
        };
        let slot = derived.author_ordinals.entry(subject.clone()).or_default();
        *slot = (*slot).max(ordinal);
        // The latest complete reply is the highest ordinal, whenever it started.
        if entry.is_none()
            && reply_content(record).is_some()
            && artifacts
                .get(&subject)
                .is_none_or(|(held, _)| *held <= ordinal)
        {
            artifacts.insert(subject, (ordinal, record));
        }
    }
    let gates: Vec<Gate<'_>> = records
        .iter()
        .filter_map(|record| {
            judged_gate(record).map(|(command, stdin, gate)| (*record, command, stdin, gate))
        })
        .collect();
    for (subject, (_, record)) in artifacts {
        let candidate = reply_content(record).unwrap_or_default();
        // The last judgment of exactly these bytes, whichever reply held them.
        let gate = gates
            .iter()
            .rfind(|(_, _, stdin, _)| *stdin == candidate)
            .map(|(_, _, _, gate)| gate.clone());
        derived.subjects.insert(
            subject,
            SubjectSeed::Artifact {
                candidate: candidate.to_string(),
                candidate_call: record.call.id.clone(),
                gate,
            },
        );
    }
    if let Some(seed) = acceptance(
        &records,
        &gates,
        &HostOwned::new(criteria, requirement_texts),
    )? {
        derived.subjects.insert("acceptance".into(), seed);
    }
    Ok(derived)
}

/// A judged gate: its record, command, stdin and what the script saw.
type Gate<'a> = (&'a WorkflowV2CallRecord, &'a str, &'a str, GateSeed);

/// An acceptance author reply the script can read: its entry, with the
/// host-owned fields of an owed check set, and its call ordinal.
struct Reply<'a> {
    record: &'a WorkflowV2CallRecord,
    id: String,
    ordinal: u64,
    text: String,
    entry: Value,
}

/// The reply round a gate's candidate holds: the latest round after which
/// the loop's entries (each entry's latest reply up to that round) are the
/// candidate's. Only rounds that leave every entry as it was (a reply that
/// repeats its entry byte for byte) share it, and which of them holds the
/// gate changes nothing: the carried entries are the same.
fn gate_round(
    candidate: &BTreeMap<String, Value>,
    replies: &[Reply<'_>],
    refuted: &std::collections::BTreeSet<String>,
) -> u64 {
    let mut state: BTreeMap<&str, &Value> = BTreeMap::new();
    let mut repaired_after_gate: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    let mut round = 0;
    for (index, reply) in replies.iter().enumerate() {
        if round > 0 && refuted.contains(&reply.id) {
            // A post-gate author call for a refuted entry is always a repair,
            // even if the author repeats byte-for-byte what the gate saw.
            repaired_after_gate.insert(reply.id.clone());
        }
        state.insert(&reply.id, &reply.entry);
        let round_ends = replies
            .get(index + 1)
            .is_none_or(|next| next.ordinal != reply.ordinal);
        if round_ends
            && repaired_after_gate.is_empty()
            && state
                .iter()
                .all(|(id, entry)| candidate.get(*id) == Some(*entry))
        {
            round = reply.ordinal;
        }
    }
    round
}

fn acceptance(
    records: &[&WorkflowV2CallRecord],
    gates: &[Gate<'_>],
    host: &HostOwned<'_>,
) -> Result<Option<SubjectSeed>> {
    let mut replies_read = Vec::new();
    let mut candidates = Vec::new();
    let mut any_reply = false;
    let mut reply_states: BTreeMap<String, (u64, bool)> = BTreeMap::new();
    for record in records {
        if let Some(stdin) = record::acceptance_candidate(record) {
            candidates.push((record.call.id.as_str(), stdin));
        }
        let Some((_, Some(id), ordinal)) = author_call(&record.call.id) else {
            continue;
        };
        let Some(content) = reply_content(record) else {
            continue;
        };
        any_reply = true;
        let text = record::extract_object(content);
        let entry = record::reply_entry_with_blocks(content, &id);
        if let Some(entry) = entry.as_ref() {
            let text = if text.contains("command_block") {
                serde_json::to_string(entry)?
            } else {
                text
            };
            let entry = host.stamp(entry.clone(), &id);
            replies_read.push(Reply {
                record,
                id: id.clone(),
                ordinal,
                text,
                entry,
            });
        }
        if reply_states
            .get(&id)
            .is_none_or(|(latest, _)| ordinal >= *latest)
        {
            reply_states.insert(id, (ordinal, entry.is_some()));
        }
    }
    // Oldest first by ordinal; one round's replies keep their start order.
    replies_read.sort_by_key(|reply| reply.ordinal);
    let mut judged = Vec::new();
    let mut refuted_ids = std::collections::BTreeSet::new();
    for gate in gates
        .iter()
        .filter(|(_, command, ..)| *command == ACCEPTANCE_GATE)
    {
        let (record, _, stdin, _) = gate;
        let entries = record::candidate_entries(stdin, &record.call.id)?;
        // Compared as the loop kept them: a candidate an older runtime
        // assembled may predate the host-owned stamp.
        let by_id: BTreeMap<String, Value> = entries
            .iter()
            .map(|(id, entry, _)| (id.clone(), host.stamp(entry.clone(), id)))
            .collect();
        let mut refuted = std::collections::BTreeSet::new();
        // A gate may explicitly refute an entry whose bytes happen to match
        // its candidate. That entry is still owed a repair in a later round;
        // it cannot make an identical later reply part of the round the gate
        // already holds.
        for finding in &gate.3.findings {
            let Some(text) = finding["text"].as_str() else {
                continue;
            };
            if let Some(id) = text
                .strip_prefix("check '")
                .and_then(|rest| rest.split_once("' was refuted").map(|(id, _)| id))
            {
                refuted.insert(id.to_string());
            }
        }
        for id in by_id.keys() {
            if refuted.contains(id.as_str()) {
                refuted_ids.insert(id.clone());
            } else {
                refuted_ids.remove(id.as_str());
            }
        }
        let round = gate_round(&by_id, &replies_read, &refuted);
        refuted_ids.extend(refuted);
        judged.push((round, gate, entries));
    }
    // A stable sort: gates of one round keep their start order.
    judged.sort_by_key(|(round, ..)| *round);
    let last = judged.last();
    let mut entries: BTreeMap<String, Value> = BTreeMap::new();
    for (call_id, stdin) in candidates {
        for (id, entry, supplementary) in record::candidate_entries(stdin, call_id)? {
            // Supplementary entries can be carried even without a current
            // owed finding; their criterion is restamped from current PRD text.
            let _ = supplementary;
            entries.insert(id, entry);
        }
    }
    // Replies of the rounds the last gate's candidate holds are its evidence;
    // later rounds are the replies since, the latest per entry.
    let after = last.map_or(0, |(round, ..)| *round);
    let mut replies: BTreeMap<String, ReplySeed> = BTreeMap::new();
    for reply in replies_read.iter().filter(|reply| reply.ordinal > after) {
        replies.insert(
            reply.id.clone(),
            ReplySeed {
                call_id: reply.record.call.id.clone(),
                id: reply.id.clone(),
                text: reply.text.to_string(),
                criterion: record::authored_criterion(reply.record, &reply.id),
            },
        );
    }
    for reply in &replies_read {
        entries
            .entry(reply.id.clone())
            .or_insert_with(|| reply.entry.clone());
    }
    let unreadable_replies = reply_states
        .into_iter()
        .filter_map(|(id, (_, readable))| {
            (!readable).then_some((
                id,
                "accepted author reply could not be reconstructed as an entry with its expected id"
                    .into(),
            ))
        })
        .collect();
    let carried_entries: Vec<Value> = entries.values().cloned().collect();
    let mut validation_entries = entries.clone();
    for reply in &replies_read {
        validation_entries.insert(reply.id.clone(), reply.entry.clone());
    }
    if last.is_none() && !any_reply && entries.is_empty() {
        return Ok(None);
    }
    let mut invalid = BTreeMap::new();
    for (id, entry) in &validation_entries {
        let refusals = record::entry_refusals(id, &host.stamp(entry.clone(), id))?;
        if !refusals.is_empty() {
            invalid.insert(id.clone(), refusals);
        }
    }
    let carried = entries.len();
    Ok(Some(SubjectSeed::Entries {
        gates: judged
            .iter()
            .map(|(_, (.., gate), _)| gate.clone())
            .collect(),
        candidate: last.map(|(_, (_, _, stdin, _), _)| stdin.to_string()),
        replies: replies.into_values().collect(),
        unreadable_replies,
        carried_entries,
        refuted_ids: refuted_ids.into_iter().collect(),
        invalid,
        carried,
    }))
}

#[cfg(test)]
#[path = "workflow_decompose_seed_derive_tests.rs"]
pub(crate) mod tests;
