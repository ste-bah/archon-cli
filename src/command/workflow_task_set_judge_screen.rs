//! Structural screening of judge evidence before it becomes an author finding.
use super::*;

use super::schema::RefutationKind;

#[path = "workflow_task_set_judge_screen_mutation.rs"]
mod mutation;

#[derive(Default)]
pub(super) struct Screen {
    reasks: BTreeMap<String, usize>,
    coverage_shown: BTreeSet<String>,
    corrections: BTreeMap<String, String>,
    resolved_roots: Option<JudgeRoots>,
}

pub(super) const TOOLCHAIN_FACT: &str = "A stale build product is a defect only when the check runs a product it did not build (for example, a prebuilt binary); a check that builds before it runs cannot be stale. The shell, PATH and executables the repository does not build are fixed and out of bounds for counterexamples.";

fn denies_state(text: &str) -> bool {
    // Conservatively recognise complete denials only. Qualifying clauses,
    // including a denial of one approach followed by a real state, survive.
    let lower = text.trim().trim_end_matches(['.', '!']).to_lowercase();
    [
        "none is constructible",
        "no counterexample",
        "no counterexample exists",
        "no valid counterexample exists",
        "no in-bounds state",
        "no in-bounds state is constructible",
        "not a counterexample",
        "not an in-bounds counterexample",
        "no state is constructible",
        "no state is needed",
        "no change is needed",
        "no counterexample is needed",
    ]
    .contains(&lower.as_str())
}

fn pure_denial(decision: &JudgeResponse) -> bool {
    decision.kind == RefutationKind::State
        && denies_state(&decision.counterexample)
        && denies_state(&decision.reason)
}

fn denial_claim(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "no counterexample",
        "no valid counterexample",
        "no in-bounds state",
        "none is constructible",
        "not a counterexample",
        "no change is needed",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn duplicate_claim(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "duplicate implementation",
        "duplicated implementation",
        "delivered duplicate",
        "duplicated gate",
        "relocated implementation",
        "another copy",
        "old copy",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn toolchain_claim(text: &str) -> bool {
    let lower = text.to_lowercase();
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "PATH")
        || [
            "stale build",
            "cached build",
            "cached binary",
            "cached executable",
            "search path",
            "shell quirk",
            "shell behaviour",
            "shell behavior",
        ]
        .iter()
        .any(|phrase| lower.contains(phrase))
}

/// The passability judge has its own schema and proof obligation. Preserve
/// that schema; only the main acceptance judge gets the structural screen.
pub(super) fn apply_unscreened(
    contract: &mut AcceptanceContract,
    content: &str,
) -> Result<BTreeSet<String>> {
    let document =
        crate::command::workflow_freeze_candidate::candidate_document(content.trim().as_bytes());
    let Ok(mut response) = serde_json::from_slice::<serde_json::Value>(&document) else {
        return apply_judgments_schema(contract, content, false);
    };
    if let Some(decisions) = response
        .get_mut("decisions")
        .and_then(serde_json::Value::as_array_mut)
    {
        for decision in decisions {
            let Some(object) = decision.as_object_mut() else {
                return apply_judgments_schema(contract, content, false);
            };
            object.insert("kind".into(), serde_json::json!("state"));
            object.insert("cited_paths".into(), serde_json::json!([]));
        }
    }
    apply_judgments_schema(contract, &serde_json::to_string(&response)?, false)
}

impl Screen {
    /// A malformed schema is left to the existing full-batch retry path.
    /// A structural fault removes only its id from this reply, so all other
    /// verdicts keep their credit and only that id is asked again.
    pub(super) fn apply(
        &mut self,
        contract: &mut AcceptanceContract,
        content: &str,
        roots: &JudgeRoots,
    ) -> Result<BTreeSet<String>> {
        let document = crate::command::workflow_freeze_candidate::candidate_document(
            content.trim().as_bytes(),
        );
        // Validate ids, required schema and prose before counting a re-ask.
        let mut validated = contract.clone();
        let mut missing = apply_judgments(&mut validated, content)?;
        require_judged_prose(&subset_without_ids(&validated, &missing))?;
        let response: BatchedJudgeResponse = serde_json::from_slice(&document)?;
        for decision in response.decisions {
            let entry = validated
                .acceptance
                .iter_mut()
                .chain(&mut validated.supplementary)
                .find(|entry| entry.id == decision.id)
                .expect("validated decision id");
            if decision.verdict == JudgeDecision::Accepted {
                self.corrections.remove(&decision.id);
                continue;
            }
            let prose = format!("{} {}", decision.counterexample, decision.reason);
            let duplicate = decision.kind == RefutationKind::Duplicate || duplicate_claim(&prose);
            let denial = pure_denial(&decision);
            // Ordinary state and accepted replies need no git or source reads.
            let resolved_roots: &JudgeRoots = if (duplicate && !decision.cited_paths.is_empty())
                || decision.kind == RefutationKind::Coverage
            {
                self.resolved_roots
                    .get_or_insert_with(|| each::with_commit(roots))
            } else {
                roots
            };
            let exact_duplicate = decision.kind == RefutationKind::Duplicate
                && decision
                    .cited_paths
                    .iter()
                    .any(|path| named_files::same_check_in_base(path, entry, resolved_roots));
            let mut prefetch = false;
            let coverage_fault = if decision.kind == RefutationKind::Coverage
                && !self.coverage_shown.contains(&decision.id)
            {
                let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
                let supplied = named_files_with_budget(entry, resolved_roots, &mut remaining);
                let missing_content = decision.cited_paths.iter().any(|path| {
                    !supplied.iter().any(|file| {
                        file["status"] == "supplied"
                            && file["path"].as_str().is_some_and(|shown| {
                                named_files::same_path(shown, path, entry, resolved_roots)
                            })
                    })
                });
                if missing_content {
                    self.coverage_shown.insert(decision.id.clone());
                    let mut remaining =
                        archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
                    let files = named_files::named_files_for_inputs_with_budget(
                        entry,
                        resolved_roots,
                        &mut remaining,
                        &decision.cited_paths,
                    );
                    prefetch = true;
                    Some(format!(
                        "Reconsider this coverage refutation with the cited named_files supplied: {}. Tests may cover less only when supplied content shows a covered requirement unasserted.",
                        serde_json::to_string(&files)?
                    ))
                } else {
                    None
                }
            } else {
                None
            };
            let fault = if coverage_fault.is_some() {
                coverage_fault
            } else if denial {
                Some("Your refutation contradicts its own evidence: it denies an in-bounds counterexample state. Describe a constructible state or accept the check.".to_string())
            } else if denial_claim(&prose) {
                Some("Reconsider the denial within this refutation and describe the complete passing-but-false state. Preserve any constructive objection.".to_string())
            } else if exact_duplicate {
                Some("The cited existing check runs exactly the same command at the same working directory. Reconsider whether this is a refutation.".to_string())
            } else if duplicate {
                Some("Reconsider the duplicate claim: name a path that exists in the base tree and explain the passing-but-false state. A duplicate word alone proves no fault in a check.".to_string())
            } else if mutation::mutation_only(&decision) {
                Some(mutation::MUTATION_FACT.to_string())
            } else if toolchain_claim(&prose) {
                Some(TOOLCHAIN_FACT.to_string())
            } else {
                None
            };
            if let Some(fault) = fault {
                let count = self.reasks.entry(decision.id.clone()).or_default();
                if prefetch || *count < 2 {
                    tracing::info!(
                        entry = %decision.id,
                        counterexample = %decision.counterexample,
                        "screened judge refutation; requesting reconsideration"
                    );
                    if !prefetch {
                        *count += 1;
                    }
                    missing.insert(decision.id.clone());
                    self.corrections.insert(decision.id.clone(), fault);
                } else {
                    if denial || exact_duplicate {
                        entry.judgment.verdict = JudgeDecision::Accepted;
                        entry.judgment.host_call_id = if denial {
                            "host:refutation-without-state"
                        } else {
                            "host:identical-duplicate-check"
                        }
                        .to_string();
                    } else {
                        tracing::info!(
                            entry = %decision.id,
                            counterexample = %decision.counterexample,
                            "forwarded persistent judge refutation"
                        );
                        entry.judgment.reason = format!(
                            "[kind={}] {}",
                            serde_json::to_value(&decision.kind)?.as_str().unwrap(),
                            entry.judgment.reason
                        );
                    }
                    self.corrections.remove(&decision.id);
                }
            } else {
                self.corrections.remove(&decision.id);
            }
            // Keep the finding form while naming evidenced duplicate paths.
            if duplicate
                && !missing.contains(&decision.id)
                && entry.judgment.verdict == JudgeDecision::Refuted
            {
                let paths = decision
                    .cited_paths
                    .iter()
                    .filter(|path| named_files::exists_in_base(path, entry, resolved_roots))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if !paths.is_empty() && !entry.judgment.reason.contains(&paths) {
                    entry.judgment.reason = format!("{paths}: {}", entry.judgment.reason);
                }
            }
        }
        rounds::merge_judged_entries(contract, &subset_without_ids(&validated, &missing));
        Ok(missing)
    }

    pub(super) fn next_ids(&self, missing: &BTreeSet<String>) -> BTreeSet<String> {
        self.corrections
            .keys()
            .find(|id| missing.contains(*id))
            .map(|id| BTreeSet::from([id.clone()]))
            .unwrap_or_else(|| missing.clone())
    }

    pub(super) fn instruction(&self, contract: &AcceptanceContract) -> String {
        self.corrections
            .iter()
            .filter(|(id, _)| {
                contract
                    .acceptance
                    .iter()
                    .chain(&contract.supplementary)
                    .any(|entry| &entry.id == *id)
            })
            .map(|(id, correction)| format!("\ncheck '{id}': {correction}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
#[path = "workflow_task_set_judge_screen_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_task_set_judge_review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "workflow_task_set_judge_safety_tests.rs"]
mod safety_tests;

#[cfg(test)]
#[path = "workflow_task_set_judge_r16_dry_run_tests.rs"]
mod r16_dry_run_tests;

#[cfg(test)]
#[path = "workflow_task_set_judge_screen_mutation_tests.rs"]
mod mutation_tests;
