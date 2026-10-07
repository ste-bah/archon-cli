//! The phase seed read from a run's own durable records (Issue 360).
//!
//! Nothing here decides acceptance. It reads, per author subject, the latest
//! candidate the old runtime produced, the gates that judged it and the
//! replies authored since, and checks every carried acceptance entry with this
//! build's freeze entry validator. The script turns that into its loop state
//! with its own rules (`workflow_decompose_v1_seed.js`). The acceptance
//! candidate it assembles goes to the gate again; a submission whose bytes a
//! recorded gate already judged keeps that recorded verdict (Issue 361).
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
//! pause. No PRD, project or task name is known here.
use std::collections::BTreeMap;

use anyhow::Result;
use archon_workflow::{WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Status};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::seed_unmapped as unmapped;
#[path = "workflow_decompose_seed_host_owned.rs"]
mod host_owned;
use host_owned::HostOwned;

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

/// The JSON object a reply carries, as the script's `extractJsonObject`
/// reads it: the first fenced block, else the text, cut to its outermost
/// braces.
fn extract_object(content: &str) -> &str {
    let raw = content.trim();
    let body = raw
        .find("```")
        .and_then(|open| {
            let rest = &raw[open + 3..];
            let rest = rest.strip_prefix("json").unwrap_or(rest).trim_start();
            rest.find("```").map(|close| &rest[..close])
        })
        .unwrap_or(raw)
        .trim();
    match (body.find('{'), body.rfind('}')) {
        (Some(first), Some(last)) if last > first => &body[first..=last],
        _ => body,
    }
}

/// The entry `id` a reply holds, as the script's `unwrapEntry` reads it.
fn reply_entry(text: &str, id: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    if parsed["id"] == id {
        return Some(parsed);
    }
    match parsed["acceptance"].as_array().map(Vec::as_slice) {
        Some([only]) if only["id"] == id => Some(only.clone()),
        _ => None,
    }
}

/// This build's freeze shape refusals of one carried entry.
fn entry_refusals(id: &str, entry: &Value) -> Result<Vec<String>> {
    let candidate = serde_json::to_vec(&serde_json::json!({ "entries": [entry] }))?;
    Ok(crate::command::workflow::element_shape_defects(
        &candidate,
        &crate::command::workflow::ENTRY_SHAPE,
    )
    .iter()
    .map(|defect| {
        format!(
            "acceptance entry '{id}' was refused: {}",
            defect.message.replacen("entries/0", "entry", 1)
        )
    })
    .collect())
}

/// The entries of a recorded acceptance candidate, in its own order.
fn candidate_entries(stdin: &str, gate: &str) -> Result<Vec<(String, Value, bool)>> {
    let field = format!("v2/results/{gate}.call.options.host_command.stdin");
    let document: Value =
        serde_json::from_str(stdin).map_err(|e| unmapped(&field, &e.to_string()))?;
    let mut out = Vec::new();
    for (list, supplementary) in [("entries", false), ("supplementary", true)] {
        let Some(items) = document.get(list) else {
            if list == "entries" {
                return Err(unmapped(&field, "the candidate has no entries list"));
            }
            continue;
        };
        for item in items.as_array().into_iter().flatten() {
            let id = item["id"]
                .as_str()
                .ok_or_else(|| unmapped(&field, "a candidate entry has no string id"))?;
            out.push((id.to_string(), item.clone(), supplementary));
        }
    }
    Ok(out)
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

pub(crate) fn derive(
    records: &[WorkflowV2CallRecord],
    pause_ids: &[String],
    criteria: &BTreeMap<String, String>,
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
    if let Some(seed) = acceptance(&records, &gates, &HostOwned::new(criteria, &gates))? {
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
    text: &'a str,
    entry: Value,
}

/// The reply round a gate's candidate holds: the latest round after which
/// the loop's entries (each entry's latest reply up to that round) are the
/// candidate's. Only rounds that leave every entry as it was (a reply that
/// repeats its entry byte for byte) share it, and which of them holds the
/// gate changes nothing: the carried entries are the same.
fn gate_round(candidate: &BTreeMap<String, Value>, replies: &[Reply<'_>]) -> u64 {
    let mut state: BTreeMap<&str, &Value> = BTreeMap::new();
    let mut round = 0;
    for (index, reply) in replies.iter().enumerate() {
        state.insert(&reply.id, &reply.entry);
        let round_ends = replies
            .get(index + 1)
            .is_none_or(|next| next.ordinal != reply.ordinal);
        if round_ends
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
    let mut any_reply = false;
    for record in records {
        let Some((_, Some(id), ordinal)) = author_call(&record.call.id) else {
            continue;
        };
        let Some(content) = reply_content(record) else {
            continue;
        };
        any_reply = true;
        let text = extract_object(content);
        if let Some(entry) = reply_entry(text, &id) {
            let entry = host.stamp(entry, &id);
            replies_read.push(Reply {
                record,
                id,
                ordinal,
                text,
                entry,
            });
        }
    }
    // Oldest first by ordinal; one round's replies keep their start order.
    replies_read.sort_by_key(|reply| reply.ordinal);
    let mut judged = Vec::new();
    for gate in gates
        .iter()
        .filter(|(_, command, ..)| *command == ACCEPTANCE_GATE)
    {
        let (record, _, stdin, _) = gate;
        let entries = candidate_entries(stdin, &record.call.id)?;
        // Compared as the loop kept them: a candidate an older runtime
        // assembled may predate the host-owned stamp.
        let by_id = entries
            .iter()
            .map(|(id, entry, _)| (id.clone(), host.stamp(entry.clone(), id)))
            .collect();
        judged.push((gate_round(&by_id, &replies_read), gate, entries));
    }
    // A stable sort: gates of one round keep their start order.
    judged.sort_by_key(|(round, ..)| *round);
    let last = judged.last();
    let mut entries: BTreeMap<String, Value> = BTreeMap::new();
    if let Some((_, (record, ..), candidate)) = last {
        for (id, entry, supplementary) in candidate.iter().cloned() {
            // A supplementary check is carried only as the check a gate said
            // is owed: the script authors it against that requirement text.
            let prefix = format!("check '{id}': PRD requirement ");
            if supplementary
                && !judged.iter().any(|(_, (.., gate), _)| {
                    gate.findings
                        .iter()
                        .any(|f| f["text"].as_str().is_some_and(|t| t.starts_with(&prefix)))
                })
            {
                return Err(unmapped(
                    &format!("v2/results/{}.supplementary.{id}", record.call.id),
                    "no recorded gate names this supplementary check as owed",
                ));
            }
            let entry = host.stamp(entry, &id);
            entries.insert(id, entry);
        }
    }
    // Replies of the rounds the last gate's candidate holds are its evidence;
    // later rounds are the replies since, the latest per entry.
    let after = last.map_or(0, |(round, ..)| *round);
    let mut replies: BTreeMap<String, ReplySeed> = BTreeMap::new();
    for reply in replies_read.iter().filter(|reply| reply.ordinal > after) {
        entries.insert(reply.id.clone(), reply.entry.clone());
        replies.insert(
            reply.id.clone(),
            ReplySeed {
                call_id: reply.record.call.id.clone(),
                id: reply.id.clone(),
                text: reply.text.to_string(),
            },
        );
    }
    if last.is_none() && !any_reply {
        return Ok(None);
    }
    let mut invalid = BTreeMap::new();
    for (id, entry) in &entries {
        let refusals = entry_refusals(id, entry)?;
        if !refusals.is_empty() {
            invalid.insert(id.clone(), refusals);
        }
    }
    Ok(Some(SubjectSeed::Entries {
        gates: judged
            .iter()
            .map(|(_, (.., gate), _)| gate.clone())
            .collect(),
        candidate: last.map(|(_, (_, _, stdin, _), _)| stdin.to_string()),
        replies: replies.into_values().collect(),
        invalid,
        carried: entries.len(),
    }))
}

#[cfg(test)]
#[path = "workflow_decompose_seed_derive_tests.rs"]
pub(crate) mod tests;
