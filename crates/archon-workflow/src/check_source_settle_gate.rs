//! What a round settles and how its judge is asked (PLAN-11, [`super`]):
//! a held change is provisional until its branch landed; each judgment is
//! recorded and replayed, so a resumed or replayed round decides exactly
//! what it decided before; and the guards that keep a check from passing on
//! nothing.

use std::collections::BTreeMap;
use std::path::Path;

use super::{SourceJudge, SourceJudgeInput, SourceVerdict};
use crate::check_source_pins::CheckSourcePins;
use crate::check_source_pins::current_bytes;
use crate::check_source_requests::{
    ORIGIN_LANDING, RequestResolution, SourceChangeRequest, requests_dir,
};
use crate::check_source_resolve::{Roots, resolve};
use crate::task_set_contract::content_digest;
use crate::write_coordinator::ItemId;
use crate::write_coordinator::patch_apply::{ApplyResumeStatus, resume_status};

pub(super) enum Landing {
    Landed,
    NotLanded(String),
    /// The branch has not finished landing (or failing) yet.
    Undecided,
}

/// Whether the branch that proposed `request` landed. A change found in the
/// tree has no branch and is always settled. A branch whose patch was
/// applied landed; one with no patch landed when it was accepted and held
/// this very change (all it did was propose the test); anything else did
/// not, and its proposal is never applied.
pub(super) fn landing_state(run_root: &Path, request: &SourceChangeRequest) -> Landing {
    if request.origin != ORIGIN_LANDING {
        return Landing::Landed;
    }
    let (call, branch) = (&request.call_id, &request.branch_id);
    match resume_status(&ItemId::from(branch.as_str()), run_root, call) {
        ApplyResumeStatus::Applied | ApplyResumeStatus::IdempotentNoop => return Landing::Landed,
        ApplyResumeStatus::PendingApply => return Landing::Undecided,
        ApplyResumeStatus::Failed(why) => {
            return Landing::NotLanded(format!("branch {branch} failed to land: {why}"));
        }
        ApplyResumeStatus::Conflicted => {
            return Landing::NotLanded(format!("branch {branch} conflicted and did not land"));
        }
        ApplyResumeStatus::NotPersisted | ApplyResumeStatus::SkippedIgnored => {}
    }
    let store = crate::WorkflowV2ResultStore::new(run_root.join("v2"));
    let outcome = match store.load_branch_outcome(call, branch) {
        Ok(Some(outcome)) => outcome,
        Ok(None) | Err(_) => return Landing::Undecided,
    };
    let accepted = matches!(
        outcome.status,
        crate::WorkflowV2Status::Accepted | crate::WorkflowV2Status::Noop
    );
    let held_here = outcome.result.as_ref().is_some_and(|result| {
        result.data["check_source_held"]
            .as_array()
            .is_some_and(|held| {
                held.iter()
                    .any(|entry| entry["request_id"] == serde_json::json!(request.request_id))
            })
    });
    if accepted && held_here {
        Landing::Landed
    } else {
        Landing::NotLanded(format!(
            "branch {branch} did not land (status {:?}); its proposal is never applied",
            outcome.status
        ))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Recorded {
    input_digest: String,
    accepted: bool,
    reason: String,
    counterexample: String,
}

fn input_digest(input: &SourceJudgeInput) -> String {
    let value = serde_json::json!([
        input.check_id,
        input.criterion,
        input.command,
        input.source,
        input.origin,
        input.pinned,
        input.pinned_retained,
        input.proposed,
        input.diff,
        input.part,
    ]);
    content_digest(value.to_string().as_bytes())
}

/// The judge's verdict on `input` for `request_id`: the recorded one when
/// this exact question was asked before, else the judge's, recorded before
/// it is used. A verdict that cannot be recorded is not used.
pub(super) async fn recorded_verdict(
    run_root: &Path,
    judge: &dyn SourceJudge,
    request_id: &str,
    input: &SourceJudgeInput,
) -> Result<SourceVerdict, String> {
    let dir = requests_dir(run_root).join("judgments");
    let part = input
        .part
        .map_or(String::new(), |(index, _)| format!(".part-{index}"));
    let path = dir.join(format!(
        "{request_id}{part}.{}.json",
        input
            .check_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            })
            .collect::<String>()
    ));
    let digest = input_digest(input);
    if let Ok(bytes) = std::fs::read(&path) {
        let recorded: Recorded = serde_json::from_slice(&bytes)
            .map_err(|error| format!("{} is unreadable: {error}", path.display()))?;
        if recorded.input_digest == digest {
            return Ok(SourceVerdict {
                accepted: recorded.accepted,
                reason: recorded.reason,
                counterexample: recorded.counterexample,
            });
        }
    }
    let verdict = judge.judge(input).await?;
    let recorded = Recorded {
        input_digest: digest,
        accepted: verdict.accepted,
        reason: verdict.reason.clone(),
        counterexample: verdict.counterexample.clone(),
    };
    crate::check_source_pins::write_atomically(
        &path,
        &serde_json::to_vec_pretty(&recorded).expect("a verdict serializes"),
    )
    .map_err(|error| format!("{} could not be recorded: {error}", path.display()))?;
    Ok(verdict)
}

pub(super) struct Guards {
    pub(super) defects: BTreeMap<String, String>,
    pub(super) must_fail: BTreeMap<String, String>,
}

/// A check that runs no source the host can pin and none of whose logic is
/// inline in the contract rests on nothing frozen: a contract defect. A
/// check that names a test nothing defines yet may run, but must not pass.
pub(super) fn guards(pins: &CheckSourcePins, roots: &Roots) -> Guards {
    let mut guards = Guards {
        defects: BTreeMap::new(),
        must_fail: BTreeMap::new(),
    };
    for (id, check) in &pins.checks {
        let now = resolve(&check.command, roots.of(check.cwd), roots);
        if check.sources.is_empty() && now.found.is_empty() && !now.inline {
            let unresolved = if now.unresolved.is_empty() {
                String::new()
            } else {
                format!(" ({})", now.unresolved.join("; "))
            };
            guards.defects.insert(
                id.clone(),
                format!(
                    "check '{id}' runs no source the host can pin and none of its logic is written in the contract{unresolved}: its outcome would rest on nothing frozen; re-author it to run a test or script in the repository"
                ),
            );
        }
        for watch in check.watches.iter().filter(|w| w.test_name.is_some()) {
            let satisfied = now
                .found
                .iter()
                .any(|found| crate::check_source_drift::satisfies(watch, found));
            if !satisfied {
                guards.must_fail.insert(
                    id.clone(),
                    format!(
                        "check '{id}' names test `{}` under {}, which nothing defines yet: a run that selects no test is not a pass",
                        watch.test_name.as_deref().unwrap_or_default(),
                        if watch.dir.is_empty() { "the repository root" } else { &watch.dir }
                    ),
                );
            }
        }
    }
    guards
}

/// The change from `before` to `after` as a unified diff, cut at line
/// boundaries into parts of at most `limit` bytes (a single longer line is
/// a part of its own, never cut). Nothing is dropped.
pub(super) fn diff_parts(before: &str, after: &str, limit: usize) -> Vec<String> {
    let diff = similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .to_string();
    let mut parts = Vec::new();
    let mut current = String::new();
    for line in diff.split_inclusive('\n') {
        if !current.is_empty() && current.len() + line.len() > limit {
            parts.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.is_empty() || parts.is_empty() {
        parts.push(current);
    }
    parts
}

pub(super) fn add(defects: &mut BTreeMap<String, String>, id: &str, why: String) {
    defects
        .entry(id.to_string())
        .and_modify(|text| {
            text.push_str("; ");
            text.push_str(&why);
        })
        .or_insert(why);
}

pub(super) fn digest_now(roots: &Roots, request: &SourceChangeRequest) -> Option<String> {
    current_bytes(roots, request.root, &request.path, request.item.as_deref())
        .map(|bytes| content_digest(&bytes))
}

pub(super) fn resolution(
    request: &SourceChangeRequest,
    verdict: &str,
    reason: String,
) -> RequestResolution {
    RequestResolution {
        request_id: request.request_id.clone(),
        verdict: verdict.to_string(),
        reason,
        counterexample: String::new(),
        applied: false,
        repinned: false,
        at: chrono::Utc::now().to_rfc3339(),
    }
}
