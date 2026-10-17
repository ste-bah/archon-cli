//! Re-run the finished decomposition's frozen checks in freeze's check copy.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_core::{config::ArchonConfig, env_vars::ArchonEnvVars};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, AcceptancePin, JudgeDecision, TASK_SKELETON_FILE,
    validate_acceptance_bundle,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CheckResult {
    pub(super) check_id: String,
    pub(super) verdict: String,
    pub(super) really_ran: bool,
    pub(super) setup_failure: bool,
    pub(super) real_failure: bool,
    pub(super) sibling_paths: Vec<SiblingResult>,
    pub(super) findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) not_run_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) regression_guard: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SiblingResult {
    pub(super) path: String,
    pub(super) commit: Option<String>,
}

pub(super) struct ReverifyChecks {
    pub(super) checks: Vec<CheckResult>,
    pub(super) task_subjects: Vec<String>,
    pub(super) accepted: bool,
    pub(super) operational_failure: bool,
}

struct CheckProgress {
    total: usize,
    started: std::collections::BTreeMap<String, std::time::Instant>,
}

impl CheckProgress {
    fn new(total: usize) -> Self {
        Self {
            total,
            started: BTreeMap::new(),
        }
    }

    fn start(&mut self, index: usize, check_id: &str) {
        self.started
            .insert(check_id.to_string(), std::time::Instant::now());
        eprintln!("{}", self.start_line(index, check_id));
    }

    fn start_line(&self, index: usize, check_id: &str) -> String {
        format!("check {index}/{} {check_id} started", self.total)
    }

    fn finish(&self, index: usize, check_id: &str, outcome: &str) {
        let elapsed = self
            .started
            .get(check_id)
            .map(std::time::Instant::elapsed)
            .unwrap_or_default()
            .as_secs_f64();
        eprintln!("{}", self.finish_line(index, check_id, outcome, elapsed));
    }

    fn finish_line(&self, index: usize, check_id: &str, outcome: &str, elapsed: f64) -> String {
        // These lines are emitted by `archon workflow reverify` from the
        // direct CLI path (workflow_decompose_reverify::reverify), not by a
        // recorded `freeze-acceptance` host command.
        format!(
            "check {index}/{} {check_id} {outcome} elapsed={elapsed:.1}s",
            self.total
        )
    }
}

#[cfg(test)]
#[path = "workflow_decompose_reverify_acceptance_tests.rs"]
mod tests;

fn frozen_check_plan(contract: &AcceptanceContract) -> Vec<(String, bool, Option<String>)> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| {
            let accepted = entry.judgment.verdict == JudgeDecision::Accepted;
            let reason = (!accepted).then(|| {
                format!(
                    "freeze does not run this entry because its frozen judgment is {:?}; only Accepted entries are executable",
                    entry.judgment.verdict
                )
            });
            (entry.id.clone(), accepted, reason)
        })
        .collect()
}

fn not_run_checks(plan: &[(String, bool, Option<String>)]) -> Vec<CheckResult> {
    plan.iter()
        .filter(|(_, accepted, _)| !accepted)
        .map(|(id, _, reason)| CheckResult {
            check_id: id.clone(),
            verdict: "not_run".into(),
            really_ran: false,
            setup_failure: false,
            real_failure: false,
            sibling_paths: Vec::new(),
            findings: Vec::new(),
            not_run_reason: reason.clone(),
            regression_guard: None,
        })
        .collect()
}

fn apply_passability_result(
    check: &mut CheckResult,
    failed: bool,
    setup_finding: Option<String>,
    baseline_finding: Option<String>,
) {
    if let Some(finding) = setup_finding {
        check.setup_failure = true;
        check.verdict = "needs_remediation".into();
        check.findings.push(finding);
    } else if failed {
        check.real_failure = true;
    } else if let Some(finding) = baseline_finding {
        check.verdict = "needs_remediation".into();
        check.findings.push(finding);
    }
}

fn check_subjects(
    entry: &archon_workflow::task_set_contract::AcceptanceCriterion,
    skeleton: &archon_workflow::task_skeleton::TaskSkeleton,
) -> Vec<String> {
    let mut subjects = skeleton
        .tasks
        .iter()
        .filter(|task| {
            entry
                .covers
                .iter()
                .chain(std::iter::once(&entry.id))
                .any(|obligation| task.implements.contains(obligation))
        })
        .map(|task| task.task_id.clone())
        .collect::<Vec<_>>();
    if subjects.is_empty() {
        subjects.push(entry.id.clone());
    }
    subjects
}

fn remediation_task_subjects(
    checks: &[CheckResult],
    contract: &AcceptanceContract,
    skeleton: &archon_workflow::task_skeleton::TaskSkeleton,
) -> Vec<String> {
    let mut subjects = Vec::new();
    for check in checks
        .iter()
        .filter(|check| check.verdict == "needs_remediation")
    {
        if let Some(entry) = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .find(|entry| entry.id == check.check_id)
        {
            subjects.extend(check_subjects(entry, skeleton));
        }
    }
    subjects
}

fn reverify_incomplete_advice(error: anyhow::Error) -> anyhow::Error {
    let Some(incomplete) = crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error)
    else {
        return error;
    };
    let message = incomplete.to_string().replace(
        "Retry the freeze with the same candidate (resume the run): it continues from the saved results",
        "run `archon workflow reverify <run>` again; saved observations and verdicts are reused",
    );
    anyhow!(message)
}

pub(super) async fn reverify_checks(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<ReverifyChecks> {
    let prd = std::fs::read(prd_path).with_context(|| format!("reading {}", prd_path.display()))?;
    let prd_text = std::str::from_utf8(&prd).context("frozen PRD is not valid UTF-8")?;
    let expected = archon_workflow::obligation_ids::acceptance_criteria(prd_text)
        .into_keys()
        .collect::<BTreeSet<_>>();
    let contract_path = task_root.join(ACCEPTANCE_CONTRACT_FILE);
    let raw: AcceptanceContract = serde_json::from_slice(
        &std::fs::read(&contract_path)
            .with_context(|| format!("reading {}", contract_path.display()))?,
    )?;
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, task_root);
    let pin: AcceptancePin = serde_json::from_slice(
        &std::fs::read(&pin_path).with_context(|| format!("reading {}", pin_path.display()))?,
    )?;
    let contract = validate_acceptance_bundle(task_root, Some(&pin), &expected)
        .map_err(|error| anyhow!(error))?;
    if contract != raw {
        return Err(anyhow!(
            "frozen acceptance contract changed while reverify loaded it"
        ));
    }
    let check_plan = frozen_check_plan(&contract);
    let mut progress = CheckProgress::new(check_plan.len());
    let accepted_ids = check_plan
        .iter()
        .filter(|(_, accepted, _)| *accepted)
        .map(|(id, _, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_slice(
        &std::fs::read(task_root.join(TASK_SKELETON_FILE))
            .with_context(|| format!("reading {}", task_root.join(TASK_SKELETON_FILE).display()))?,
    )?;
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    eprintln!("{}", resume.progress.line());
    for (index, (id, accepted, _)) in check_plan.iter().enumerate() {
        if *accepted {
            progress.start(index + 1, id);
        }
    }
    let probe =
        crate::command::workflow_task_set::executability::HostProbe::for_reverify_with_limits(
            project_root,
            task_root,
            config.tools.cargo.to_limits(),
            &resume,
        );
    let (evidence, commit, missing, mut baseline_findings) = if accepted_ids.is_empty() {
        (Vec::new(), None, BTreeMap::new(), BTreeMap::new())
    } else {
        probe
            .run_reverify_copy(&contract, &accepted_ids)
            .await
            .map_err(|error| anyhow!("could not prepare frozen acceptance checks: {error}"))?
    };
    let regression_guards = probe.take_regression_guards();
    let mut by_id = evidence
        .into_iter()
        .map(|item| (item.acceptance_id.clone(), item))
        .collect::<BTreeMap<_, _>>();
    let mut missing = missing;
    for id in accepted_ids.difference(&by_id.keys().cloned().collect()) {
        missing
            .entry(id.clone())
            .or_insert_with(|| "the check-copy probe returned no result".to_string());
    }
    let mut operational_failure =
        !accepted_ids.is_empty() && (commit.is_none() || !missing.is_empty());
    let mut baseline_failures = BTreeMap::new();
    let mut results = not_run_checks(&check_plan);
    let check_indexes = check_plan
        .iter()
        .enumerate()
        .map(|(index, (id, _, _))| (id.as_str(), index + 1))
        .collect::<BTreeMap<_, _>>();
    for id in &accepted_ids {
        let Some(item) = by_id.remove(id) else {
            results.push(CheckResult {
                check_id: id.clone(),
                verdict: "operational_failure".into(),
                really_ran: false,
                setup_failure: false,
                real_failure: false,
                sibling_paths: Vec::new(),
                findings: vec![
                    missing
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "the check-copy probe returned no result".into()),
                ],
                not_run_reason: None,
                regression_guard: None,
            });
            progress.finish(
                *check_indexes.get(id.as_str()).unwrap_or(&0),
                id,
                "setup failure",
            );
            continue;
        };
        let classification = item.classification.clone();
        let failed = classification
            .as_ref()
            .is_some_and(|classification| !classification.passed);
        let really_ran = item.operational_error.is_none() && classification.is_some();
        let siblings = item
            .copy_layout
            .as_ref()
            .map(|layout| {
                layout
                    .siblings
                    .iter()
                    .map(|sibling| SiblingResult {
                        path: sibling.path.to_string_lossy().replace('\\', "/"),
                        commit: sibling.commit.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !really_ran {
            operational_failure = true;
        }
        if failed {
            baseline_failures.insert(id.clone(), item.clone());
        }
        results.push(CheckResult {
            check_id: id.clone(),
            verdict: if really_ran {
                "accepted"
            } else {
                "operational_failure"
            }
            .into(),
            really_ran,
            setup_failure: false,
            real_failure: false,
            sibling_paths: siblings,
            findings: Vec::new(),
            not_run_reason: None,
            regression_guard: regression_guards.get(id).cloned(),
        });
        let outcome = if let Some(basis) = regression_guards.get(id) {
            format!("passed; regression guard: {basis}")
        } else if probe.reused_check(id) {
            "reused from cache".to_string()
        } else if !really_ran {
            "setup failure".to_string()
        } else if failed {
            "failed".to_string()
        } else {
            "passed".to_string()
        };
        progress.finish(*check_indexes.get(id.as_str()).unwrap_or(&0), id, &outcome);
    }

    let mut setup_findings = BTreeMap::new();
    let failed_ids = baseline_failures.keys().cloned().collect::<BTreeSet<_>>();
    if !baseline_failures.is_empty() {
        eprintln!(
            "reverify judge phase started checks={}",
            baseline_failures.len()
        );
        let scope = crate::command::workflow_task_set::reauthor::AuthorScope::for_task_set(
            project_root,
            task_root,
            prd_path,
        )
        .map_err(|error| anyhow!(error))?;
        let client = crate::command::workflow::reverify_acceptance_client(
            config,
            env_vars,
            project_root,
            "workflow-reverify-acceptance",
            &scope,
        )
        .await?;
        let runs = crate::command::workflow_task_set::executability::BaselineRuns {
            reuse: probe.take_reuse_audit(),
            commit: commit.ok_or_else(|| anyhow!("check-copy has no commit to judge"))?,
            failures: baseline_failures,
            environment: BTreeMap::new(),
            forwarded: Vec::new(),
        };
        let mut judged = contract.clone();
        let findings = crate::command::workflow_task_set::passability::judge_baseline_failures(
            project_root,
            task_root,
            prd_text,
            client.as_ref(),
            &mut judged,
            crate::command::workflow_task_set::coverage_gate::PreImplementation {
                findings: Vec::new(),
                baseline: Some(runs),
            },
            &resume,
        )
        .await
        .map_err(reverify_incomplete_advice)?;
        for finding in findings {
            setup_findings.insert(finding.subject, finding.text);
        }
    }
    for check in &mut results {
        apply_passability_result(
            check,
            failed_ids.contains(&check.check_id),
            setup_findings.remove(&check.check_id),
            baseline_findings.remove(&check.check_id),
        );
    }
    let mut task_subjects = remediation_task_subjects(&results, &contract, &skeleton);
    task_subjects.sort();
    task_subjects.dedup();
    let accepted = !operational_failure && results.iter().all(|check| check.verdict == "accepted");
    Ok(ReverifyChecks {
        checks: results,
        task_subjects,
        accepted,
        operational_failure,
    })
}
