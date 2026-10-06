//! PLAN-11: an acceptance round settles every change to a pinned check
//! source before any check runs, so no check is ever judged by a source
//! nobody judged.
//!
//! 1. A source found changed in the tree (edited outside a landing) is
//!    recorded as a request like a held landing change
//!    (`check_source_requests::ORIGIN_ACCEPTANCE_DRIFT`).
//! 2. Each pending request goes to the judge with the check's criterion and
//!    command, the pinned version and the proposed one. Accepted: a held
//!    landing change is applied to the tree (and committed, in a Git
//!    repository, so later worktrees are cut from it) and every check it
//!    serves is re-pinned to it, with a `RepinLink`. Refused: a held change
//!    stays out; a change found in the tree is restored to its pinned bytes.
//!    A proposal the tree moved past is stale: its proposer proposes again.
//! 3. Whatever still differs from its pin afterwards -- no judge this round,
//!    a judge that failed, a pinned version that cannot be restored -- makes
//!    its checks contract defects: they fail and do not run.

use std::collections::BTreeMap;
use std::path::Path;

use crate::check_source_drift::tree_drift;
use crate::check_source_pins::{CheckSourcePins, PinStore, current_bytes};
use crate::check_source_requests::{
    self as requests, NewRequest, ORIGIN_ACCEPTANCE_DRIFT, ORIGIN_LANDING, RequestResolution,
    SourceChangeRequest, VERDICT_ACCEPTED, VERDICT_ORPHANED, VERDICT_REFUTED, VERDICT_STALE,
};
use crate::check_source_resolve::Roots;
use crate::task_set_contract::{AcceptanceContract, content_digest};

/// One part of a source shown to its judge: the text, and which part of how
/// many it is (`None` when the source is shown whole).
type JudgedPart = (Option<String>, Option<(usize, usize)>);
/// Largest source, either version, the judge is shown whole; a bigger one
/// is judged by its change, as a unified diff in parts of this size.
pub const MAX_JUDGED_SOURCE_BYTES: usize = 256 * 1024;

/// What the judge is shown for one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceJudgeInput {
    pub check_id: String,
    pub criterion: String,
    pub command: String,
    pub source: String,
    pub origin: String,
    /// The pinned version; `None` when the source did not exist when pinned
    /// (or, `pinned_retained` false, when its bytes were not kept).
    pub pinned: Option<String>,
    pub pinned_retained: bool,
    /// The proposed version; `None` deletes the source.
    pub proposed: Option<String>,
    /// For a source too large to show whole: the change as a unified diff,
    /// one part of `part` (index, count); `pinned` and `proposed` are then
    /// not shown.
    pub diff: Option<String>,
    pub part: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceVerdict {
    pub accepted: bool,
    pub reason: String,
    pub counterexample: String,
}

#[async_trait::async_trait]
pub trait SourceJudge: Send + Sync {
    async fn judge(&self, input: &SourceJudgeInput) -> Result<SourceVerdict, String>;
}

pub struct Settle<'a> {
    pub run_root: &'a Path,
    pub roots: Roots<'a>,
    pub store: &'a PinStore,
    pub contract: &'a AcceptanceContract,
    pub judge: Option<&'a dyn SourceJudge>,
    /// Why there is no judge, when there is none.
    pub judge_note: String,
}

/// One request's settlement this round (`resolution` `None`: left pending).
#[derive(Debug, Clone)]
pub struct Settlement {
    pub request: SourceChangeRequest,
    pub resolution: Option<RequestResolution>,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct Settled {
    pub pins: CheckSourcePins,
    pub settlements: Vec<Settlement>,
    /// Per check id: why it cannot run this round.
    pub defects: BTreeMap<String, String>,
    /// Per check id: why it may run but must not pass (a test it names that
    /// nothing defines yet).
    pub must_fail: BTreeMap<String, String>,
    /// Why the request store could not be read: the round cannot vouch for
    /// any check.
    pub errors: Vec<String>,
    /// A publish of the task set left a journal no settlement could settle
    /// when a re-pin was written (Issue 338): the round pauses the run.
    pub paused: Option<String>,
}

pub async fn settle(ctx: &Settle<'_>, mut pins: CheckSourcePins) -> Settled {
    let mut defects: BTreeMap<String, String> = BTreeMap::new();
    for change in tree_drift(&pins, &ctx.roots) {
        let proposed = current_bytes(
            &ctx.roots,
            change.root,
            &change.path,
            change.item.as_deref(),
        );
        let file = change
            .item
            .as_ref()
            .and_then(|_| std::fs::read(ctx.roots.of(change.root).join(&change.path)).ok());
        let recorded = requests::record(
            ctx.run_root,
            NewRequest {
                origin: ORIGIN_ACCEPTANCE_DRIFT,
                check_ids: change.check_ids.clone(),
                root: change.root,
                path: &change.path,
                item: change.item.as_deref(),
                was_pinned: change.was_pinned,
                pinned_digest: change.pinned.clone(),
                proposed: proposed.as_deref(),
                proposed_file: file.as_deref(),
                landed_file_digest: None,
                call_id: "",
                branch_id: "",
                task_ids: Vec::new(),
            },
        );
        if let Err(error) = recorded {
            for id in &change.check_ids {
                add(
                    &mut defects,
                    id,
                    format!(
                        "pinned source {} differs from its pin and the change could not be recorded: {error}",
                        change.label()
                    ),
                );
            }
        }
    }
    let (mut settlements, mut errors, mut paused) = (Vec::new(), Vec::new(), None);
    let queue = requests::pending(ctx.run_root).unwrap_or_else(|error| {
        errors.push(error);
        Vec::new()
    });
    for request in queue {
        // A held change is provisional until its branch landed.
        let settlement = match gate::landing_state(ctx.run_root, &request) {
            gate::Landing::Undecided => continue,
            gate::Landing::NotLanded(why) => Settlement {
                resolution: Some(resolution(&request, VERDICT_ORPHANED, why.clone())),
                request,
                note: why,
            },
            gate::Landing::Landed => settle_one(ctx, &mut pins, request, &mut paused).await,
        };
        if let Some(resolution) = &settlement.resolution
            && let Err(error) = requests::settle_record(ctx.run_root, resolution)
        {
            for id in &settlement.request.check_ids {
                add(&mut defects, id, error.clone());
            }
        }
        if settlement.resolution.is_none() {
            for id in &settlement.request.check_ids {
                add(&mut defects, id, settlement.note.clone());
            }
        }
        settlements.push(settlement);
        if paused.is_some() {
            break;
        }
    }
    for change in tree_drift(&pins, &ctx.roots) {
        for id in &change.check_ids {
            if defects.contains_key(id) {
                continue;
            }
            add(
                &mut defects,
                id,
                format!(
                    "pinned source {} of check '{id}' differs from its pin (pinned {}, now {}) and was not settled this round; the check does not run on a source nobody judged",
                    change.label(),
                    change.pinned.as_deref().unwrap_or("absent"),
                    change.actual.as_deref().unwrap_or("absent")
                ),
            );
        }
    }
    let guards = gate::guards(&pins, &ctx.roots);
    for (id, why) in guards.defects {
        add(&mut defects, &id, why);
    }
    Settled {
        pins,
        settlements,
        defects,
        must_fail: guards.must_fail,
        errors,
        paused,
    }
}

async fn settle_one(
    ctx: &Settle<'_>,
    pins: &mut CheckSourcePins,
    request: SourceChangeRequest,
    paused: &mut Option<String>,
) -> Settlement {
    let now = digest_now(&ctx.roots, &request);
    // A held landing change applies over the pinned source; a change found
    // in the tree is judged as the tree holds it.
    let expected = match request.origin.as_str() {
        ORIGIN_LANDING => request.pinned_digest.clone(),
        _ => request.proposed_digest.clone(),
    };
    // A new test function lands with the file it was proposed in: that file
    // must still be the one that landed around it.
    let file_now = std::fs::read(ctx.roots.of(request.root).join(&request.path))
        .ok()
        .map(|bytes| content_digest(&bytes));
    // Already applied by an earlier settlement that stopped before it was
    // recorded (a crash between the commit and the record): settled again,
    // idempotently, from its recorded verdict -- never called stale.
    let already_applied = request.origin == ORIGIN_LANDING
        && now == request.proposed_digest
        && (request.pinned_digest.is_some()
            || request.item.is_none()
            || file_now == request.proposed_file_digest);
    let file_moved = request.origin == ORIGIN_LANDING
        && request.item.is_some()
        && request.pinned_digest.is_none()
        && file_now != request.landed_file_digest;
    if !already_applied && (now != expected || file_moved) {
        let reason = format!(
            "{} now hashes to {}, not the {} the proposal was made over{}; the proposer proposes again on its next landing",
            request.label(),
            now.as_deref().unwrap_or("absent"),
            expected.as_deref().unwrap_or("absent"),
            if file_moved {
                ", or its file changed since it landed"
            } else {
                ""
            }
        );
        return Settlement {
            resolution: Some(resolution(&request, VERDICT_STALE, reason.clone())),
            request,
            note: reason,
        };
    }
    // A source that exists but could not be read is not a deletion.
    if request.proposed_digest.is_none() && ctx.roots.of(request.root).join(&request.path).exists()
    {
        return pending(
            request,
            "the source exists but could not be read, so the change cannot be judged".into(),
        );
    }
    let Some(judge) = ctx.judge else {
        return pending(
            request,
            format!("this round has no judge to decide it ({})", ctx.judge_note),
        );
    };
    let blobs = requests::blobs(ctx.run_root);
    let lookup = |digest: &Option<String>| -> Option<Option<Vec<u8>>> {
        match digest {
            None => Some(None),
            Some(digest) => blobs
                .get(digest)
                .or_else(|| ctx.store.blobs.get(digest))
                .map(Some),
        }
    };
    let Some(proposed) = lookup(&request.proposed_digest) else {
        return pending(
            request,
            "its proposed bytes are not in the request store".into(),
        );
    };
    let pinned = lookup(&request.pinned_digest);
    // A link is never a source: refused without asking the judge.
    if proposed
        .as_deref()
        .is_some_and(|bytes| bytes.starts_with(crate::check_source_pins::SYMLINK_MARK))
    {
        let mut settled = resolution(
            &request,
            VERDICT_REFUTED,
            "a check source may not be replaced by a symbolic link".into(),
        );
        return match refuse(ctx, &request, pinned) {
            Ok(applied) => {
                settled.applied = applied;
                let note = settled.reason.clone();
                Settlement {
                    request,
                    resolution: Some(settled),
                    note,
                }
            }
            Err(error) => pending(request, error),
        };
    }
    let text = |bytes: &Option<Vec<u8>>| {
        bytes
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    };
    let too_big = [proposed.as_ref(), pinned.as_ref().and_then(Option::as_ref)]
        .into_iter()
        .flatten()
        .any(|bytes| bytes.len() > MAX_JUDGED_SOURCE_BYTES);
    // A source too large to show whole is judged by its change, in parts no
    // larger than the judge is shown; every part must be accepted.
    let parts: Vec<JudgedPart> = if too_big {
        let before = pinned.as_ref().and_then(text).unwrap_or_default();
        let after = text(&proposed).unwrap_or_default();
        let chunks = gate::diff_parts(&before, &after, MAX_JUDGED_SOURCE_BYTES);
        let count = chunks.len();
        (chunks.into_iter().enumerate())
            .map(|(index, chunk)| (Some(chunk), Some((index + 1, count))))
            .collect()
    } else {
        vec![(None, None)]
    };
    let mut verdicts = Vec::new();
    for id in &request.check_ids {
        let Some(entry) = (ctx.contract.acceptance.iter())
            .chain(&ctx.contract.supplementary)
            .find(|entry| &entry.id == id)
        else {
            continue;
        };
        let command = pins
            .checks
            .get(id)
            .map(|c| c.command.clone())
            .unwrap_or_default();
        let mut combined: Option<SourceVerdict> = None;
        for (diff, part) in &parts {
            let input = SourceJudgeInput {
                check_id: id.clone(),
                criterion: entry.criterion.clone(),
                command: command.clone(),
                source: request.label(),
                origin: request.origin.clone(),
                pinned: if diff.is_some() {
                    None
                } else {
                    pinned.as_ref().and_then(text)
                },
                pinned_retained: pinned.is_some(),
                proposed: if diff.is_some() {
                    None
                } else {
                    text(&proposed)
                },
                diff: diff.clone(),
                part: *part,
            };
            let verdict = match gate::recorded_verdict(
                ctx.run_root,
                judge,
                &request.request_id,
                &input,
            )
            .await
            {
                Ok(verdict) => verdict,
                Err(error) => return pending(request, format!("the judge failed: {error}")),
            };
            let refused = !verdict.accepted;
            combined = Some(match combined {
                Some(previous) if !previous.accepted => previous,
                _ => verdict,
            });
            if refused {
                break;
            }
        }
        if let Some(verdict) = combined {
            verdicts.push((id.clone(), verdict));
        }
    }
    // No check left to judge it for: never refuted, restored or committed.
    if verdicts.is_empty() {
        let reason = format!(
            "no check it serves ({}) is in the contract any more; dropped untouched",
            request
                .check_ids
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Settlement {
            resolution: Some(resolution(&request, VERDICT_ORPHANED, reason.clone())),
            request,
            note: reason,
        };
    }
    let accepted = verdicts.iter().all(|(_, v)| v.accepted);
    let reason = verdicts
        .iter()
        .map(|(id, v)| format!("{id}: {}", v.reason))
        .collect::<Vec<_>>()
        .join("; ");
    let counterexample = verdicts
        .iter()
        .filter(|(_, v)| !v.accepted)
        .map(|(id, v)| format!("{id}: {}", v.counterexample))
        .collect::<Vec<_>>()
        .join("; ");
    let mut settled = resolution(
        &request,
        if accepted {
            VERDICT_ACCEPTED
        } else {
            VERDICT_REFUTED
        },
        reason,
    );
    settled.counterexample = counterexample;
    let outcome = if accepted {
        accept(ctx, pins, &request, proposed.as_deref(), &settled.reason)
    } else {
        refuse(ctx, &request, pinned).map_err(ApplyError::from)
    };
    match outcome {
        Ok(applied) => {
            settled.applied = applied;
            settled.repinned = accepted;
            let note = format!(
                "{} {}: {}",
                request.label(),
                settled.verdict,
                settled.reason
            );
            Settlement {
                request,
                resolution: Some(settled),
                note,
            }
        }
        Err(ApplyError::Unsettled(evidence)) => {
            *paused = Some(evidence.clone());
            pending(request, evidence)
        }
        Err(ApplyError::Failed(error)) => pending(request, error),
    }
}

fn pending(request: SourceChangeRequest, why: String) -> Settlement {
    let note = format!(
        "pinned source {} of check(s) {} has a change pending judgment (request {}): {why}; the check does not run on an unjudged source",
        request.label(),
        request
            .check_ids
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        request.request_id
    );
    Settlement {
        request,
        resolution: None,
        note,
    }
}

#[path = "check_source_settle_apply.rs"]
mod apply;
#[path = "check_source_settle_gate.rs"]
mod gate;
use apply::{ApplyError, accept, refuse};
use gate::{add, digest_now, resolution};

#[cfg(test)]
#[path = "check_source_settle_tests.rs"]
mod tests;
