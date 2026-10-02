//! Issue-117: the residual gaps an ACCEPTED remediation verifier recorded,
//! accounted before acceptance instead of dropped.
//!
//! # The gap this closes
//!
//! A verifier that accepts can still record `residual_gaps`, and nothing in
//! the run read them: the terminal rule judges the review accounting, the
//! remediation outcome and the acceptance round, none of which carries them.
//! Live on wf-0ddadd81 a cross-task unit over four tasks was accepted by a
//! verifier that recorded, as HIGH, that a provider store module reads the
//! wrong segment of a dataset id at named lines -- a file no task declares,
//! whose consistency one of those tasks' own contract requires. No branch
//! could be dispatched to fix it and the run could go green over it.
//!
//! # What the host concludes, and from what
//!
//! Population: the records this session recorded or replayed of remediation
//! VERIFY calls that ran an agent and accepted (review, cross-task,
//! contest, re-verification and escalated rounds alike), less the host's own
//! residual rounds. EVERY gap they carry is in scope (Batch O): the HOST
//! sets its severity ([`ResidualSeverity::parse`]), never the recorder's
//! label -- a high label is HIGH, anything else (low, minor, info, note,
//! nit, review, none) is MEDIUM, so no label ever drops a gap. Only the
//! host's own environment and operational records are not work
//! (`host_environment_gap`).
//!
//! Each in-scope gap is mapped through the paths it names
//! ([`residual_paths::named_files`]) and the universe's ownership:
//!
//! - it names a file a task declares: routed to those tasks (an OWNED
//!   round), which may also be granted the unowned files it names;
//! - it names only files no task declares: an EXPANSION round of the tasks
//!   it relates to ([`residual_paths::related_tasks`]), granted exactly the
//!   files the host may open ([`residual_paths::expandable`]);
//! - anything else -- no file named, ownership unprovable, nothing openable
//!   -- is ADJUDICATED, whatever its severity: one read-only verification of
//!   the recording unit's tasks on the tree as it is, after every file round;
//! - only a gap with no universe task to adjudicate it against is REPORTED,
//!   and a reported gap blocks at the final gate (`residual_gate`).
//!
//! A review remediation unit whose latest verifier refused the fix over
//! blocker files no task declares (Issue-107 escalates only into files
//! another task owns) is planned the same way, as a REVIEW round of the
//! unit's tasks and the tasks naming those files.
//!
//! Gaps are grouped into rounds per task set, at most [`MAX_GAPS_PER_ROUND`]
//! each; a pass plans EVERY round it can build (no round cap). The plan rides on the
//! pre-acceptance checkpoint's view under [`RESIDUAL_GAPS_KEY`], computed at
//! the moment of asking and never persisted; each round is ONE bounded round
//! (`maxRounds: 1`, no escalation), asked at most once per run: its done
//! checkpoint ([`done_checkpoint_id`]) marks it attempted, and an attempted
//! round is never dispatched again (`residual_dispatch`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use super::residual_paths::{TaskTexts, expandable, owners, related_tasks};
use super::{
    WorkflowV2CallRecord, WorkflowV2HostMethod, is_reusable_status, remediation_contract,
    remediation_contract_string,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// The checkpoint option that asks the host for the residual plan.
pub const RESIDUAL_GAPS_MARKER: &str = "residualGaps";
/// Key of the plan in that checkpoint's view (never `residual_gaps`: every
/// envelope carries that field of its own, and the script reads the first).
pub const RESIDUAL_GAPS_KEY: &str = "residual_plan";
/// The contract key that marks a host-planned residual round.
pub const RESIDUAL_CONTRACT_KEY: &str = "residual";
/// The write item field naming the unowned files the round may write.
pub const RESIDUAL_ITEM_PATHS_KEY: &str = "residual_expansion_paths";

/// Most gaps one round carries; a larger group is split, so a round's
/// claim is never cut to fit a prompt.
const MAX_GAPS_PER_ROUND: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResidualSeverity {
    Medium,
    High,
}

impl ResidualSeverity {
    /// The severity the HOST gives a gap: HIGH for a high label, MEDIUM for
    /// every other label or none. A recorder's low-impact label (`low`,
    /// `minor`, `info`, `note`, `nit`, `trivial`, `review`, ...) is its own
    /// judgment of a gap it still recorded, so it never drops the gap: the
    /// gap is planned like any medium one, and only a round's verifier (or an
    /// adjudicator) can resolve it.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(|raw| raw.trim().to_ascii_lowercase()).as_deref() {
            Some(
                "critical" | "blocker" | "blocking" | "high" | "major" | "severe" | "error" | "p0"
                | "p1",
            ) => Self::High,
            _ => Self::Medium,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
        }
    }
}

/// Whether a recorder labelled its gap low-impact. The label never lowers
/// the gap's severity ([`ResidualSeverity::parse`]: it is MEDIUM, planned
/// and weighed); it only keeps a gap that shares nothing but a file with a
/// round's gap from being read as that gap again (`residual_gate_rounds`).
pub(super) fn low_impact_label(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|raw| raw.trim().to_ascii_lowercase()).as_deref(),
        Some("review" | "info" | "informational" | "note" | "low" | "minor" | "trivial" | "nit")
    )
}

/// One in-scope gap a verifier recorded (a refused one's too, in later passes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Residual {
    pub recorded_by: String,
    pub id: String,
    pub severity: ResidualSeverity,
    pub description: String,
    /// The exact existing repository files its text names.
    pub files: Vec<String>,
    /// The universe tasks of the unit whose verifier recorded it.
    pub unit_tasks: BTreeSet<String>,
    /// The recording verifier's own summary.
    pub recorded_summary: String,
    /// Built by the host itself from its own records (a routed red test).
    pub host_built: bool,
}

impl Residual {
    pub fn key(&self) -> String {
        let digest = fnv64(&self.description);
        format!("{}#{}#{}", self.recorded_by, self.id, &digest[..8])
    }

    pub fn label(&self) -> String {
        let id = if self.id.is_empty() {
            "<unnamed>"
        } else {
            &self.id
        };
        format!(
            "`{id}` ({}, recorded by `{}`)",
            self.severity.as_str(),
            self.recorded_by
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundKind {
    Owned,
    Expansion,
    Review,
    /// A read-only verification of HIGH gaps no file round can carry.
    Adjudication,
}

impl RoundKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::Expansion => "expansion",
            Self::Review => "review",
            Self::Adjudication => "adjudication",
        }
    }
}

/// One host-planned bounded round.
#[derive(Debug, Clone)]
pub struct PlannedRound {
    pub key: String,
    pub kind: RoundKind,
    pub tasks: BTreeSet<String>,
    /// The files no task declares that the round may write.
    pub files: BTreeSet<String>,
    pub residuals: Vec<Residual>,
    /// A review round: the unit key its refused verdict's contract names.
    pub unit_key: Option<String>,
    /// A review round: the refusal it answers, in the host's own words.
    pub refusal: Option<Value>,
    /// The residual pass that planned it (1, 2 or 3); see `round_claim`.
    pub pass: u8,
}

impl PlannedRound {
    pub fn severity(&self) -> ResidualSeverity {
        self.residuals
            .iter()
            .map(|residual| residual.severity)
            .max()
            .unwrap_or(ResidualSeverity::High)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResidualPlan {
    pub rounds: Vec<PlannedRound>,
    /// In-scope gaps no round carries, with why.
    pub reported: Vec<(Residual, String)>,
}

/// Whether `call` belongs to a host-planned residual round.
pub fn is_residual_round(call: &super::WorkflowV2HostCall) -> bool {
    remediation_contract(call).is_some_and(|contract| contract.get(RESIDUAL_CONTRACT_KEY).is_some())
}

/// A remediation verifier AGENT that accepted, as recorded.
pub fn accepted_verdict(record: &WorkflowV2CallRecord) -> bool {
    record.invalidated_by.is_none()
        && remediation_contract_string(&record.call, "stage") == Some("verify")
        && record.call.method != WorkflowV2HostMethod::Checkpoint
        && is_reusable_status(record.status)
}

/// The plan for `records` -- this session's records at the slot, or the
/// executed calls before it at the final gate: the same function both ways.
pub fn plan_from(
    records: &[&WorkflowV2CallRecord],
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> ResidualPlan {
    let mut residuals: Vec<Residual> = records
        .iter()
        .filter(|record| accepted_verdict(record) && !is_residual_round(&record.call))
        .flat_map(|record| residuals_of(record, root))
        .collect();
    residuals.sort_by_key(Residual::key);
    residuals.dedup_by_key(|residual| residual.key());
    let (Some(universe), Some(root)) = (universe, root) else {
        let why = "the host has no task universe or repository root to map it through";
        return ResidualPlan {
            rounds: Vec::new(),
            reported: residuals
                .into_iter()
                .map(|r| (r, why.to_string()))
                .collect(),
        };
    };
    let ids: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect();
    let texts = TaskTexts::read(universe, root);
    let mut plan = ResidualPlan::default();
    let mut groups: BTreeMap<Vec<String>, Vec<(Residual, BTreeSet<String>)>> = BTreeMap::new();
    let mut adjudicate: BTreeMap<Vec<String>, Vec<Residual>> = BTreeMap::new();
    for mut residual in residuals {
        residual.unit_tasks.retain(|task| ids.contains(task));
        match route(&residual, universe, root, &texts) {
            Ok((tasks, files)) => {
                groups
                    .entry(tasks.into_iter().collect())
                    .or_default()
                    .push((residual, files));
            }
            // A gap no file round can carry, of any severity, is
            // adjudicated: one read-only verification of the recording
            // unit's tasks on the tree as it is, after every file round.
            Err(_) if !residual.unit_tasks.is_empty() => {
                let tasks: Vec<String> = residual.unit_tasks.iter().cloned().collect();
                adjudicate.entry(tasks).or_default().push(residual);
            }
            Err(why) => plan.reported.push((residual, why)),
        }
    }
    for (tasks, members) in groups {
        // A group larger than a round holds is split, each round granted
        // only the files its own gaps need.
        for chunk in members.chunks(MAX_GAPS_PER_ROUND) {
            let files: BTreeSet<String> = chunk.iter().flat_map(|(_, f)| f.clone()).collect();
            let residuals: Vec<Residual> = chunk.iter().map(|(r, _)| r.clone()).collect();
            let kind = if files.is_empty() {
                RoundKind::Owned
            } else {
                RoundKind::Expansion
            };
            plan.rounds
                .push(round(kind, tasks.clone(), files, residuals, None, None));
        }
    }
    for refused in refused_units(records) {
        if let Some(round) = review_round(refused, universe, root, &texts, &ids) {
            plan.rounds.push(round);
        }
    }
    // Every round is planned, HIGH ones first: no round cap ever turns a
    // gap into a report.
    plan.rounds
        .sort_by(|a, b| b.severity().cmp(&a.severity()).then(a.key.cmp(&b.key)));
    // Adjudications run last, after every file round has landed.
    let mut adjudications: Vec<PlannedRound> = adjudicate
        .into_iter()
        .flat_map(|(tasks, residuals)| {
            residuals
                .chunks(MAX_GAPS_PER_ROUND)
                .map(|chunk| {
                    round(
                        RoundKind::Adjudication,
                        tasks.clone(),
                        BTreeSet::new(),
                        chunk.to_vec(),
                        None,
                        None,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    adjudications.sort_by(|a, b| a.key.cmp(&b.key));
    plan.rounds.extend(adjudications);
    plan
}

/// Where one gap goes: the tasks and granted files of its round, or why it
/// has none.
fn route(
    residual: &Residual,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    texts: &TaskTexts,
) -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
    if residual.files.is_empty() {
        return Err("it names no existing repository file".to_string());
    }
    let mut owned = BTreeSet::new();
    let mut unowned = BTreeSet::new();
    for file in &residual.files {
        let declared_by = owners(universe, file, root);
        if declared_by.is_empty() {
            unowned.insert(file.clone());
        } else {
            owned.extend(declared_by);
        }
    }
    if !owned.is_empty() {
        let granted = expandable(universe, &owned, &unowned, root);
        return Ok((owned, granted));
    }
    let listed = unowned.iter().cloned().collect::<Vec<_>>().join(", ");
    let tasks: BTreeSet<String> = unowned
        .iter()
        .flat_map(|file| related_tasks(&residual.unit_tasks, &texts.naming(file, root)))
        .collect();
    if tasks.is_empty() {
        return Err(format!("no task declares {listed} and none relates to it"));
    }
    let granted = expandable(universe, &tasks, &unowned, root);
    if granted.is_empty() {
        return Err(format!(
            "no task declares {listed}, and the host may not open it: its ownership is unprovable, it is a protected path, or a task forbids it"
        ));
    }
    Ok((tasks, granted))
}

pub(super) fn round(
    kind: RoundKind,
    tasks: Vec<String>,
    files: BTreeSet<String>,
    residuals: Vec<Residual>,
    unit_key: Option<String>,
    refusal: Option<Value>,
) -> PlannedRound {
    let identity = format!(
        "{}|{}|{}|{}|{}",
        kind.as_str(),
        tasks.join("+"),
        files.iter().cloned().collect::<Vec<_>>().join("+"),
        residuals
            .iter()
            .map(Residual::key)
            .collect::<Vec<_>>()
            .join("+"),
        unit_key.as_deref().unwrap_or_default()
    );
    PlannedRound {
        key: format!("residual-{}", fnv64(&identity)),
        kind,
        tasks: tasks.into_iter().collect(),
        files,
        residuals,
        unit_key,
        refusal,
        pass: 1,
    }
}

pub(super) fn finished(record: &WorkflowV2CallRecord) -> i64 {
    chrono::DateTime::parse_from_rfc3339(&record.finished_at)
        .map(|at| at.timestamp_nanos_opt().unwrap_or(i64::MIN))
        .unwrap_or(i64::MIN)
}

fn fnv64(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[path = "residual_wording.rs"]
mod wording;
use wording::{Wording, worded};
#[path = "residual_dispositions.rs"]
mod dispositions;
pub use dispositions::GAP_DISPOSITIONS_KEY;
#[path = "residual_gaps.rs"]
mod gaps;
pub use gaps::{host_environment_gap, residuals_of};
#[path = "residual_review.rs"]
mod review;
use review::{refused_units, review_round};
#[path = "residual_view.rs"]
mod view;
pub use view::{
    disposition_instruction, done_checkpoint_id, is_residual_slot, residual_plan_view, round_claim,
    round_view, second_pass_view, session_records, third_pass_view, with_residual_plan,
};

#[path = "residual_dispatch.rs"]
mod dispatch;
pub use dispatch::{refused_residual_result, residual_refusal};
#[path = "residual_gate.rs"]
mod gate;
pub use gate::{ResidualVerdict, residual_verdict, tip_owed_commands};

#[path = "residual_second_pass.rs"]
mod second_pass;
pub use second_pass::{
    REFUSED_RED_GAP_ID, RESIDUAL_PASS_KEY, is_second_pass_round, is_second_pass_slot,
    second_pass_plan,
};

#[path = "residual_owed.rs"]
mod owed;
#[path = "residual_superseded.rs"]
mod superseded;

#[path = "residual_third_pass.rs"]
mod third_pass;
pub use third_pass::{is_third_pass_round, is_third_pass_slot, third_pass_plan};

#[path = "residual_later_pass.rs"]
mod later_pass;
pub use later_pass::{
    DEFAULT_MAX_RESIDUAL_PASSES, is_later_pass_round, later_pass_plan, pass_plans,
    recording_moved_on, round_pass, slot_pass,
};
#[path = "residual_regression.rs"]
mod regression;
pub use regression::{REGRESSION_GAP_ID, is_regression_gap};

#[cfg(test)]
#[path = "residual_plan_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "residual_gate_tests.rs"]
mod gate_tests;

#[cfg(test)]
#[path = "residual_disposition_tests.rs"]
mod disposition_tests;

#[cfg(test)]
#[path = "residual_second_pass_tests.rs"]
mod second_pass_tests;

#[cfg(test)]
#[path = "residual_second_pass_gate_tests.rs"]
mod second_pass_gate_tests;

#[cfg(test)]
#[path = "residual_third_pass_tests.rs"]
mod third_pass_tests;

#[cfg(test)]
#[path = "residual_uncapped_tests.rs"]
mod uncapped_tests;
