use std::collections::{BTreeMap, BTreeSet};

use archon_workflow::task_set_contract::{AcceptanceContract, AcceptanceCriterion, JudgeDecision};
use archon_workflow::v2::acceptance_stage::coverage::supplementary_requirement;

pub(super) fn refuted_ids(entry: &AcceptanceCriterion) -> Vec<String> {
    if entry.judgment.verdict != JudgeDecision::Refuted {
        return Vec::new();
    }
    let named: BTreeSet<_> = entry
        .judgment
        .reason
        .split(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
        .collect();
    entry
        .covers
        .iter()
        .filter(|id| named.contains(id.as_str()))
        .cloned()
        .collect()
}

pub(super) fn refuted_by_check(contract: &AcceptanceContract) -> BTreeMap<String, Vec<String>> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| (entry.id.clone(), refuted_ids(entry)))
        .collect()
}

pub(super) fn validate_covers(
    candidate: AcceptanceCriterion,
    frozen: &AcceptanceCriterion,
    refuted: &[String],
    routed: &BTreeSet<String>,
) -> Result<AcceptanceCriterion, String> {
    let before: BTreeSet<_> = frozen.covers.iter().collect();
    let after: BTreeSet<_> = candidate.covers.iter().collect();
    if !after.is_subset(&before) {
        return Err("covers may only retain or drop existing ids".into());
    }
    // Only a minted entry (one whose frozen covers hold its own id) must keep
    // it; a legacy free-form SUP id names no requirement of its own.
    if let Some(required) = supplementary_requirement(&frozen.id)
        .filter(|required| frozen.covers.iter().any(|id| id.trim() == *required))
        && !candidate.covers.iter().any(|id| id == required)
    {
        return Err(format!(
            "covers must retain the entry's own requirement '{required}'"
        ));
    }
    if after != before {
        for id in before.difference(&after) {
            if !refuted.contains(id) {
                return Err(format!(
                    "covered id '{id}' was not named in the judge's refutation reason; retain it"
                ));
            }
            if !routed.contains(*id) {
                return Err(format!(
                    "covered id '{id}' has no repair route yet; retain it"
                ));
            }
        }
    }
    Ok(candidate)
}

pub(super) fn validate(
    candidate: AcceptanceCriterion,
    frozen: &AcceptanceCriterion,
    refuted: &[String],
    routed: &BTreeSet<String>,
) -> Result<AcceptanceCriterion, String> {
    let candidate = validate_covers(candidate, frozen, refuted, routed)?;
    let before: BTreeSet<_> = frozen.covers.iter().collect();
    let after: BTreeSet<_> = candidate.covers.iter().collect();
    if candidate.check == frozen.check && after == before {
        return Err(format!(
            "check '{}': the reply repeats the check being replaced; change it so it resolves every finding above",
            frozen.id
        ));
    }
    Ok(candidate)
}

pub(super) struct Repair {
    refuted: BTreeMap<String, Vec<String>>,
    routed: BTreeSet<String>,
}

impl Repair {
    pub(super) fn new(contract: &AcceptanceContract, routed: BTreeSet<String>) -> Self {
        Self {
            refuted: refuted_by_check(contract),
            routed,
        }
    }

    pub(super) fn update(&mut self, candidate: &AcceptanceCriterion) {
        self.refuted
            .get_mut(&candidate.id)
            .expect("named id")
            .extend(refuted_ids(candidate));
    }

    pub(super) fn droppable(&self, frozen: &AcceptanceCriterion) -> Vec<String> {
        frozen
            .covers
            .iter()
            .filter(|id| self.refuted[&frozen.id].contains(id) && self.routed.contains(*id))
            .cloned()
            .collect()
    }

    pub(super) fn cached(
        &self,
        candidate: AcceptanceCriterion,
        frozen: &AcceptanceCriterion,
        refused: &[std::path::PathBuf],
        working: &AcceptanceContract,
        scope: &super::AuthorScope,
    ) -> Result<Option<AcceptanceCriterion>, String> {
        let candidate = super::author::live_root_refusal(candidate, refused)?;
        match validate_covers(candidate, frozen, &self.refuted[&frozen.id], &self.routed) {
            Ok(candidate) => {
                super::check_defects::check_defects(working, candidate, scope).map(Some)
            }
            Err(reason) => {
                eprintln!(
                    "acceptance re-author ignored saved check {} (host cache miss): {reason}",
                    frozen.id
                );
                Ok(None)
            }
        }
    }

    pub(super) fn repair(
        &self,
        reply: &str,
        frozen: &AcceptanceCriterion,
        markers: &[String],
    ) -> Result<AcceptanceCriterion, String> {
        super::reply_parse::repair_entry(
            reply,
            frozen,
            markers,
            &self.refuted[&frozen.id],
            &self.routed,
        )
    }
}

pub(super) fn prompt_coverage(
    current: &mut serde_json::Value,
    frozen: &AcceptanceCriterion,
    scope: &super::AuthorScope,
    droppable: &[String],
) -> anyhow::Result<()> {
    current["covers"] = serde_json::json!(frozen.covers);
    let texts = scope.covered_texts()?;
    let covers_text: BTreeMap<_, _> = frozen
        .covers
        .iter()
        .map(|id| (id, texts.get(id.trim()).map_or("", String::as_str)))
        .collect();
    current["covers_text"] = serde_json::json!(covers_text);
    current["droppable_covers"] = serde_json::json!(droppable);
    Ok(())
}

pub(super) fn feedback(
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    gate: &super::ReauthorGate<'_>,
    crashed: &BTreeMap<String, String>,
) -> BTreeMap<String, Vec<String>> {
    let mut feedback = BTreeMap::new();
    for id in ids {
        let frozen = super::entry(contract, id).expect("named ids were checked above");
        let first = if let Some(seed) = gate.seeds.get(id).or_else(|| crashed.get(id)) {
            seed.clone()
        } else if frozen.judgment.verdict == JudgeDecision::Accepted {
            format!(
                "check '{id}' was named for re-authoring; replace it with a check that fails whenever the criterion is false"
            )
        } else {
            super::judge::author_note(frozen)
        };
        feedback.insert(id.clone(), vec![first]);
    }
    feedback
}
#[cfg(test)]
pub(super) fn author_prompt(
    scope: &super::AuthorScope,
    frozen: &AcceptanceCriterion,
    notes: &[String],
    attempt: usize,
) -> String {
    super::author::author_prompt_with_prefetch(
        scope,
        frozen,
        notes,
        attempt,
        &super::author::prefetch_files(scope, frozen, notes),
        &[],
    )
    .expect("read requirement texts")
}

pub(super) fn judge_roots(scope: &super::AuthorScope) -> anyhow::Result<super::judge::JudgeRoots> {
    Ok(super::judge::JudgeRoots {
        project: Some(scope.project_root.clone()),
        repository: Some(scope.repository_root.clone()),
        task_root: Some(scope.task_root.clone()),
        covered_texts: scope.covered_texts()?,
        repository_commit: scope
            .baseline
            .as_ref()
            .map(|baseline| baseline.commit.clone()),
    })
}
