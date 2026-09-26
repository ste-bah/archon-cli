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
//! residual rounds. Of their gaps, those of severity high (`critical`,
//! `blocker`, `high`) or medium (`medium`, `moderate`) are in scope; the
//! host's own bookkeeping gaps carry `review`/`info` and are not.
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
//!   -- is REPORTED by name at the final gate (`residual_gate`).
//!
//! A review remediation unit whose latest verifier refused the fix over
//! blocker files no task declares (Issue-107 escalates only into files
//! another task owns) is planned the same way, as a REVIEW round of the
//! unit's tasks and the tasks naming those files.
//!
//! Gaps are grouped into one round per task set. The plan rides on the
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

/// Most rounds one plan holds; gaps beyond them are reported.
const MAX_ROUNDS: usize = 6;
/// Characters kept of a gap's description in the plan.
const DESCRIPTION_CHARS: usize = 1_200;
/// Characters kept of a refused review verdict's summary and prompt.
const REFUSAL_CHARS: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResidualSeverity {
    Medium,
    High,
}

impl ResidualSeverity {
    /// The in-scope severity a gap declares; `None` for the host's own
    /// bookkeeping severities and the low-impact list. A severity the gate
    /// cannot read, or none at all, is MEDIUM: never silently dropped.
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        let Some(raw) = raw.map(|raw| raw.trim().to_ascii_lowercase()) else {
            return Some(Self::Medium);
        };
        match raw.as_str() {
            "critical" | "blocker" | "blocking" | "high" | "major" | "severe" | "error" | "p0"
            | "p1" => Some(Self::High),
            "review" | "info" | "informational" | "note" | "low" | "minor" | "trivial" | "nit" => {
                None
            }
            _ => Some(Self::Medium),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
        }
    }
}

/// One in-scope gap an accepted verifier recorded.
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
    let mut groups: BTreeMap<Vec<String>, (BTreeSet<String>, Vec<Residual>)> = BTreeMap::new();
    let mut adjudicate: BTreeMap<Vec<String>, Vec<Residual>> = BTreeMap::new();
    for mut residual in residuals {
        residual.unit_tasks.retain(|task| ids.contains(task));
        match route(&residual, universe, root, &texts) {
            Ok((tasks, files)) => {
                let group = groups.entry(tasks.into_iter().collect()).or_default();
                group.0.extend(files);
                group.1.push(residual);
            }
            // A HIGH gap no file round can carry is adjudicated: one
            // read-only verification of the recording unit's tasks on the
            // tree as it is, after every file round.
            Err(_)
                if residual.severity == ResidualSeverity::High
                    && !residual.unit_tasks.is_empty() =>
            {
                let tasks: Vec<String> = residual.unit_tasks.iter().cloned().collect();
                adjudicate.entry(tasks).or_default().push(residual);
            }
            Err(why) => plan.reported.push((residual, why)),
        }
    }
    for (tasks, (files, residuals)) in groups {
        let kind = if files.is_empty() {
            RoundKind::Owned
        } else {
            RoundKind::Expansion
        };
        plan.rounds
            .push(round(kind, tasks, files, residuals, None, None));
    }
    for refused in refused_units(records) {
        if let Some(round) = review_round(refused, universe, root, &texts, &ids) {
            plan.rounds.push(round);
        }
    }
    plan.rounds
        .sort_by(|a, b| b.severity().cmp(&a.severity()).then(a.key.cmp(&b.key)));
    let why = format!("the host plans at most {MAX_ROUNDS} residual rounds of a kind per run");
    for extra in plan.rounds.split_off(plan.rounds.len().min(MAX_ROUNDS)) {
        plan.reported
            .extend(extra.residuals.into_iter().map(|r| (r, why.clone())));
    }
    // Adjudications run last, after every file round has landed.
    let mut adjudications: Vec<PlannedRound> = adjudicate
        .into_iter()
        .map(|(tasks, residuals)| {
            round(
                RoundKind::Adjudication,
                tasks,
                BTreeSet::new(),
                residuals,
                None,
                None,
            )
        })
        .collect();
    adjudications.sort_by(|a, b| a.key.cmp(&b.key));
    for extra in adjudications.split_off(adjudications.len().min(MAX_ROUNDS)) {
        plan.reported
            .extend(extra.residuals.into_iter().map(|r| (r, why.clone())));
    }
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
    }
}

pub(super) fn finished(record: &WorkflowV2CallRecord) -> i64 {
    chrono::DateTime::parse_from_rfc3339(&record.finished_at)
        .map(|at| at.timestamp_nanos_opt().unwrap_or(i64::MIN))
        .unwrap_or(i64::MIN)
}

pub(super) fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}...", text.chars().take(limit).collect::<String>())
}

fn fnv64(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[path = "residual_gaps.rs"]
mod gaps;
pub use gaps::{flagged_of, residuals_of};

#[path = "residual_review.rs"]
mod review;
use review::{refused_units, review_round};

#[path = "residual_view.rs"]
mod view;
pub use view::{
    done_checkpoint_id, is_residual_slot, residual_plan_view, round_view, session_records,
    with_residual_plan,
};

#[path = "residual_dispatch.rs"]
mod dispatch;
pub use dispatch::{refused_residual_result, residual_refusal};

#[path = "residual_gate.rs"]
mod gate;
pub use gate::{ResidualVerdict, residual_verdict};

#[cfg(test)]
#[path = "residual_plan_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "residual_gate_tests.rs"]
mod gate_tests;
