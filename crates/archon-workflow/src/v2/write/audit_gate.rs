//! Consume only host-persisted findings; a disposition is not resolution.
//!
//! The prompt half (`preamble`) and the check half (`EvidenceContext::judge`)
//! are ONE contract, kept in this file so they cannot drift apart again.
//! Issue-14, live on wf-7db01ce7 `agents-3-0`: the preamble asked for
//! `evidence_paths` and said nothing about what one may be; the check
//! required every path to be a file the branch changed, and a correct
//! disposition that cited the audit's own equivalent (unchanged, as it must
//! be) was voided wholesale — manifest `Failed`, every dependent wave skipped.
//!
//! The ownership half (`judge_branch`) reads the [`ScopeGrant`] the three
//! ownership gates accepted the branch under, never the assignment's declared
//! list alone. Issue-15, live on wf-719ff3b0 `agents-4-0`: the audit flagged
//! two declared files as `exists_elsewhere` / `wire_or_migrate`; the coder
//! migrated code out of the two equivalents — the action the audit demands —
//! and both were unclaimed by any other item, granted, declared in the
//! manifest and recorded in `data.scope_granted`. This check, still handed
//! `assignment.owned_targets`, rejected the branch for changing them.
use super::worktree_scope_grant::ScopeGrant;
use super::*;
use crate::repository_audit::{AuditRecord, AuditReport, RequiredAction};
use serde::Deserialize;

use crate::repository_audit::reuse::load_state as state;

/// The evidence rule, told to the agent verbatim and enforced by `judge`.
pub const AUDIT_EVIDENCE_RULE: &str = "An evidence path is accepted only if it is \
(a) a file this branch changed, created or deleted, (b) the declared_path itself, \
(c) one of the finding's equivalents, or (d) an existing path in the repository worktree. \
At least one evidence path must be a file this branch changed, created or deleted. \
Any other path is dropped from the entry and reported; the entry stands if what remains \
still satisfies the rule.";

pub(super) fn preamble(store: &WorkflowV2ResultStore, paths: &[String]) -> WorkflowResult<String> {
    let Some(state) = state(store)? else {
        return Ok(String::new());
    };
    let Some(report) = state.ledger.history.last() else {
        return Ok(String::new());
    };
    let mut lines = Vec::new();
    let mut contested = Vec::new();
    for record in report
        .records
        .iter()
        .filter(|r| paths.contains(&r.declared_path))
    {
        let mut line = serde_json::json!({"snapshot":report.snapshot,"declared_path":record.declared_path,
            "verdict":record.verdict,"equivalents":record.equivalents,"required_action":record.required_action,"operator_waived":state.ledger.is_waived(&record.declared_path,&report.snapshot)});
        // Issue-112: named only when contested, so every other line is as it was.
        if let Some(contest) = state
            .ledger
            .contested(&report.snapshot)
            .into_iter()
            .find(|contest| contest.declared_path == record.declared_path)
        {
            line["contested"] = serde_json::json!(contest.describe());
            contested.push(record.declared_path.clone());
        }
        lines.push(serde_json::to_string(&line)?);
    }
    if lines.is_empty() {
        return Ok(String::new());
    }
    let contest_rule = if contested.is_empty() {
        String::new()
    } else {
        format!(
            "\nContested paths ({}): the tasks that declare them disagree, and a verified landing of one of them left them as they are. Do not create, restore, edit or delete them; the host holds the run on the contradiction until every declaring task's own verification agrees.",
            contested.join(", ")
        )
    };
    Ok(format!(
        "\nHost repository audit (equivalents are read context, NOT write permission):\n{}\n{}{contest_rule}\n",
        lines.join("\n"),
        contract_text()
    ))
}

/// What a disposition is, stated once, in the same words the check uses.
fn contract_text() -> String {
    format!(
        "Audit disposition contract: for every finding above whose required_action is \
\"wire_or_migrate\" and operator_waived is false, return exactly one entry in \
data.audit_dispositions of the form {{\"declared_path\": <the finding's declared_path>, \
\"snapshot\": <the finding's snapshot, verbatim>, \"explanation\": <what this branch did \
about the finding; non-empty, at most 2048 characters>, \"evidence_paths\": \
[<repository-relative paths>]}}. {AUDIT_EVIDENCE_RULE} A finding with no such entry, or \
with more than one, fails this branch. Dispositions are proposals, not resolution; \
actual applied changes are assessed independently."
    )
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disposition {
    pub(super) declared_path: String,
    pub(super) snapshot: String,
    pub(super) explanation: String,
    pub(super) evidence_paths: Vec<String>,
}

fn parse_dispositions(result: &WorkflowV2Result) -> Vec<Disposition> {
    serde_json::from_value(
        result
            .data
            .get("audit_dispositions")
            .cloned()
            .unwrap_or(serde_json::json!([])),
    )
    .unwrap_or_default()
}

/// Everything the evidence rule reads, so `enforce` (pre-apply, against the
/// branch worktree) and `applied_dispositions` (post-apply, against the
/// canonical tree) judge by the same rule.
pub(super) struct EvidenceContext<'a> {
    pub(super) manifest: Option<&'a PatchManifest>,
    pub(super) records: &'a [AuditRecord],
    pub(super) worktree: &'a Path,
}

/// A disposition that stands, with the paths the rule dropped from it.
pub(super) struct Judged {
    pub(super) disposition: Disposition,
    pub(super) dropped: Vec<String>,
}

impl EvidenceContext<'_> {
    /// Rule (a): a file this branch changed, created or deleted.
    fn touched(&self, path: &str) -> bool {
        self.manifest.is_some_and(|m| {
            m.changed_files
                .iter()
                .chain(&m.created_files)
                .chain(&m.deleted_files)
                .any(|p| p == path)
        })
    }
    /// Rule (d): an existing path in the worktree, named repository-relative.
    /// An absolute path or one that climbs out of the tree is never evidence.
    fn present_in_worktree(&self, path: &str) -> bool {
        let relative = Path::new(path);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return false;
        }
        self.worktree.join(relative).exists()
    }
    /// The disposition with invalid evidence paths dropped, or `None` when it
    /// does not stand: wrong snapshot, empty or oversized explanation, or no
    /// remaining path that is real work of this branch.
    pub(super) fn judge(&self, disposition: Disposition, snapshot: &str) -> Option<Judged> {
        if disposition.snapshot != snapshot
            || disposition.explanation.trim().is_empty()
            || disposition.explanation.len() > 2048
        {
            return None;
        }
        let equivalents = self
            .records
            .iter()
            .find(|r| r.declared_path == disposition.declared_path)
            .map(|r| r.equivalents.as_slice())
            .unwrap_or(&[]);
        let (kept, dropped): (Vec<String>, Vec<String>) =
            disposition.evidence_paths.iter().cloned().partition(|p| {
                self.touched(p)
                    || *p == disposition.declared_path
                    || equivalents.contains(p)
                    || self.present_in_worktree(p)
            });
        if !kept.iter().any(|p| self.touched(p)) {
            return None;
        }
        Some(Judged {
            disposition: Disposition {
                evidence_paths: kept,
                ..disposition
            },
            dropped,
        })
    }
}

fn dropped_note(declared_path: &str, dropped: &[String]) -> String {
    format!(
        "audit disposition for {declared_path}: evidence path(s) dropped as neither changed by this branch, the declared path, an audit equivalent, nor present in the worktree: {}",
        dropped.join(", ")
    )
}

/// The dispositions an applied manifest may credit: same rule as `enforce`,
/// read against the canonical tree the patch was applied to.
pub(super) fn applied_dispositions(
    result: &WorkflowV2Result,
    manifest: &PatchManifest,
    snapshot: &str,
    records: &[AuditRecord],
    canonical_root: &Path,
) -> Vec<Disposition> {
    let context = EvidenceContext {
        manifest: Some(manifest),
        records,
        worktree: canonical_root,
    };
    parse_dispositions(result)
        .into_iter()
        .filter(|d| manifest.declared_target_files.contains(&d.declared_path))
        .filter_map(|d| context.judge(d, snapshot).map(|judged| judged.disposition))
        .collect()
}

/// What `judge_branch` decided: the findings left unanswered or the changes
/// left unauthorised (`gaps`, each one a rejection), and what a reviewer is
/// told about the ones that stand (`notes`, each one a `Review` evidence line).
#[derive(Default)]
pub(super) struct AuditJudgement {
    pub(super) gaps: Vec<String>,
    pub(super) notes: Vec<String>,
}

fn granted_equivalent_note(declared_path: &str, equivalent: &str) -> String {
    format!(
        "audit finding for {declared_path}: equivalent {equivalent} was changed under the wave \
         scope grant — unclaimed by any other item in the wave, declared in the manifest as \
         granted (data.scope_granted); the audit's mention of it granted nothing"
    )
}

/// Judge one branch's dispositions and changes against the audit report.
///
/// `declared` is the assignment's declared targets: the list the preamble
/// was built from, so exactly the findings the agent was told to answer. A
/// granted path that happens to carry a finding of its own is not among
/// them, and no disposition is demanded for it.
///
/// `grant` is the scope the three ownership gates accepted the branch under:
/// the declared targets plus every unclaimed changed path it was granted. A
/// changed equivalent is judged by THAT and by nothing else. The audit naming
/// an equivalent gives the branch no permission to change it — an audit
/// mention never widens scope; only the explicit grant does, and the grant is
/// what put the path into the manifest's `declared_target_files`. So a
/// changed equivalent that was granted is authorised and named for review, a
/// changed equivalent inside the declared plan is the branch's own file, and
/// any other changed equivalent — contested by another item in the wave, or
/// changed without being reported — is rejected exactly as before.
pub(super) fn judge_branch(
    report: &AuditReport,
    waived: &dyn Fn(&str) -> bool,
    dispositions: &[Disposition],
    context: &EvidenceContext<'_>,
    declared: &[String],
    grant: &ScopeGrant,
) -> AuditJudgement {
    let mut judgement = AuditJudgement::default();
    let flagged = report
        .records
        .iter()
        .filter(|r| declared.contains(&r.declared_path));
    for record in flagged
        .clone()
        .filter(|r| r.required_action == RequiredAction::WireOrMigrate && !waived(&r.declared_path))
    {
        let mut matches = dispositions
            .iter()
            .filter(|d| d.declared_path == record.declared_path);
        // Exactly one entry per flagged path, as the contract says.
        let judged = match (matches.next(), matches.next()) {
            (Some(one), None) => context.judge(one.clone(), &report.snapshot),
            _ => None,
        };
        match judged {
            Some(judged) => {
                if !judged.dropped.is_empty() {
                    judgement
                        .notes
                        .push(dropped_note(&record.declared_path, &judged.dropped));
                }
            }
            None => judgement.gaps.push(record.declared_path.clone()),
        }
    }
    if context.manifest.is_none() {
        return judgement;
    }
    for record in flagged {
        for equivalent in record.equivalents.iter().filter(|e| context.touched(e)) {
            if grant.is_granted(equivalent) {
                judgement
                    .notes
                    .push(granted_equivalent_note(&record.declared_path, equivalent));
            } else if !grant.covers(equivalent) {
                judgement.gaps.push(format!(
                    "{equivalent} (equivalent is outside declared ownership)"
                ));
            }
        }
    }
    judgement
}

/// Pre-apply audit gate for one accepted branch, judged against the branch
/// worktree: `declared` is what the preamble showed the agent, `grant` what
/// the ownership gates accepted its changes under (see [`judge_branch`]).
pub(super) fn enforce(
    store: &WorkflowV2ResultStore,
    declared: &[String],
    grant: &ScopeGrant,
    worktree: &Path,
    result: &mut WorkflowV2Result,
    manifest: &mut Option<PatchManifest>,
) -> WorkflowResult<()> {
    if !matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return Ok(());
    }
    let Some(state) = state(store)? else {
        return Ok(());
    };
    let Some(report) = state.ledger.history.last() else {
        return Err(WorkflowError::StateCorrupt(
            "audit ledger has no assessment".into(),
        ));
    };
    let dispositions = parse_dispositions(result);
    let context = EvidenceContext {
        manifest: manifest.as_ref(),
        records: &report.records,
        worktree,
    };
    let waived = |path: &str| state.ledger.is_waived(path, &report.snapshot);
    let AuditJudgement { gaps, notes } =
        judge_branch(report, &waived, &dispositions, &context, declared, grant);
    for note in notes {
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            note,
        ));
    }
    if gaps.is_empty() {
        return Ok(());
    }
    let reason = format!(
        "repository audit rejected unexplained or unauthorized changes: {}",
        gaps.join(", ")
    );
    if let Some(m) = manifest.as_mut() {
        m.status = ManifestStatus::Failed {
            reason: reason.clone(),
        };
        let path = PathBuf::from(manifest_path_for(
            store.root().parent().unwrap(),
            &m.stage_id,
            &m.item_id,
        ));
        std::fs::write(&path, serde_json::to_vec_pretty(m)?)
            .map_err(|e| WorkflowError::io(path, e))?;
    }
    // The manifest is withdrawn, so the wave keeps the worktree's work as
    // partial work (`branch_keeps_partial_work`) for the next attempt at
    // these tasks: the work was judged, the disposition was not.
    *manifest = None;
    result.status = WorkflowV2Status::NeedsReview;
    result.summary = reason.clone();
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "repository_audit_unaddressed".into(),
        description: reason,
        severity: Some("blocking".into()),
    });
    Ok(())
}

#[cfg(test)]
#[path = "audit_gate_tests.rs"]
mod tests;
