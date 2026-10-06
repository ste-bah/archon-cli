//! ACC-A7: the sanctioned republish, extended to ADD checks.
//!
//! A frozen contract that falls short of its PRD -- an acceptance id with no
//! check, a requirement no check covers (every contract frozen before
//! `covers` existed covers none) -- is completed mid-run: the host authors
//! each owed check (`workflow_live_v3_acceptance_author`), and this module
//! publishes them as a successor of the frozen chain, the way a per-check
//! repair is published: under the chain lock, re-verified, every gate in the
//! mode its stage was frozen in, the replaced chain filed by digest, one
//! lineage link, one atomic transaction.
//!
//! Host-validated, never taken on trust: every added entry must be new to
//! the contract, accepted by exactly the freeze-time judge (model and
//! provider) every kept check records, and -- for an owed supplementary
//! check -- cover the requirement it is owed for. Every kept entry stays
//! byte-identical.
//!
//! The PRD may have moved since the freeze (it gained the ids): the contract
//! is then rebound to the PRD as it is now, and the rebind -- from and to
//! digest -- is written into the lineage link's trigger. An obligation the
//! moved PRD added that no skeleton task claims is claimed by every task of
//! the set together, as a recorded shared claim (the reroute rule: a check
//! no task owns goes to every task), so the skeleton gate judges the set it
//! would have judged had the PRD said so at the freeze. Whatever a gate
//! still refuses stops the republish, and the caller records it by name.

use archon_workflow::task_set_contract::{AcceptanceCriterion, JudgeDecision};
use archon_workflow::v2::acceptance_stage::coverage::supplementary_requirement;

use super::*;

/// What an extension adds to the frozen chain.
pub(crate) struct Extension {
    /// The authored, judged, probed entries to add.
    pub(crate) entries: Vec<AcceptanceCriterion>,
    /// The PRD's digest as it is now.
    pub(crate) prd_digest: String,
    /// The digest the contract was frozen against.
    pub(crate) frozen_prd_digest: String,
    /// Obligations of the PRD as it is now that the skeleton's tasks claim
    /// none of: claimed by every task together.
    pub(crate) new_obligations: BTreeSet<String>,
}

impl Extension {
    /// What the lineage link's trigger records beside the ids.
    pub(super) fn note(&self) -> String {
        let mut note = format!(
            "extension adds or re-authors {}",
            (self.entries.iter())
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if self.prd_digest != self.frozen_prd_digest {
            note.push_str(&format!(
                "; the contract is rebound from PRD digest {} to {}",
                self.frozen_prd_digest, self.prd_digest
            ));
        }
        if !self.new_obligations.is_empty() {
            note.push_str(&format!(
                "; obligations {} are claimed by every task of the set together (a shared claim: no task claimed them)",
                self.new_obligations
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        note
    }

    /// Every task claims each new obligation, as a shared claim.
    pub(super) fn claim_new_obligations(&self, skeleton: &mut TaskSkeleton) {
        for task in &mut skeleton.tasks {
            for obligation in &self.new_obligations {
                if !task.implements.contains(obligation) {
                    task.implements.push(obligation.clone());
                }
            }
        }
    }
}

/// The recorded judge an entry's judgment names, as (model, provider).
fn judged_by(entry: &AcceptanceCriterion) -> Option<(String, String)> {
    let sampling = entry.judgment.sampling.as_ref()?;
    Some((
        sampling["model"].as_str()?.to_string(),
        sampling["provider"].as_str()?.to_string(),
    ))
}

/// Add `extension.entries` to the chain frozen beside `request.tasks_root`
/// and republish it (see the module doc). `request.ids` must be exactly the
/// added ids.
pub(crate) fn extend_and_republish(
    request: ReauthorRequest<'_>,
    extension: &Extension,
) -> Result<ReauthorResult> {
    archon_workflow::stage_write::mapped(|| extend_owned(request, extension), anyhow::Error::from)
}

fn extend_owned(request: ReauthorRequest<'_>, extension: &Extension) -> Result<ReauthorResult> {
    let tasks_root = request.tasks_root;
    let _lock = ChainLock::acquire(
        &acceptance_pin_path(request.project_root, tasks_root),
        tasks_root,
    )?;
    let adding: BTreeSet<String> = (extension.entries.iter())
        .map(|entry| entry.id.clone())
        .collect();
    let verified = verify::verify_with(&request, Some(&adding))?;
    if verified.contract.prd.digest != extension.frozen_prd_digest {
        return Err(anyhow!(
            "the frozen contract names PRD digest {}, not {}: the chain moved while the checks were authored",
            verified.contract.prd.digest,
            extension.frozen_prd_digest
        ));
    }
    let prd_digest = content_digest(verified.prd_text.as_bytes());
    if prd_digest != extension.prd_digest {
        return Err(anyhow!(
            "the PRD moved while the checks were authored (digest {prd_digest}, authored against {}); they are authored against it again",
            extension.prd_digest
        ));
    }
    for entry in &extension.entries {
        if entry.judgment.verdict != JudgeDecision::Accepted {
            return Err(anyhow!(
                "added check '{}' is not accepted by the judge",
                entry.id
            ));
        }
        if judged_by(entry).as_ref() != Some(&verified.judge) {
            return Err(anyhow!(
                "added check '{}' was not judged by the freeze-time judge {}/{}",
                entry.id,
                verified.judge.0,
                verified.judge.1
            ));
        }
        if let Some(requirement) = supplementary_requirement(&entry.id)
            && !entry.covers.iter().any(|covered| covered == requirement)
        {
            return Err(anyhow!(
                "added supplementary check '{}' does not cover {requirement}, the requirement it is owed for",
                entry.id
            ));
        }
    }
    // Kept entries stay byte-identical and in place; a re-authored one is
    // replaced where it stands (M3/M4); added ones follow.
    let mut repaired = verified.contract.clone();
    for entry in &extension.entries {
        let held = (repaired.acceptance.iter_mut())
            .chain(&mut repaired.supplementary)
            .find(|held| held.id == entry.id);
        match held {
            Some(held) => *held = entry.clone(),
            None if supplementary_requirement(&entry.id).is_some() => {
                repaired.supplementary.push(entry.clone())
            }
            None => repaired.acceptance.push(entry.clone()),
        }
    }
    repaired.prd.digest = prd_digest;
    super::publish_chain_owned(&request, verified, repaired, Vec::new(), Some(extension))
}

#[cfg(test)]
#[path = "workflow_acceptance_republish_extend_tests.rs"]
mod tests;
