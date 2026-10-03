//! Recheck resumed entries with absent or stale passability receipts.

use super::*;
use crate::command::workflow_task_set::{
    executability::HostUnproven, passability::cannot_pass_findings,
};

/// Every resumed entry clears the same gate as a freshly authored one.
/// Keep an unstamped entry until it has been probed: a valid check need not
/// be authored again, and a refuted one carries its evidence to the author.
#[allow(clippy::too_many_arguments)]
pub(super) async fn validate(
    site: &Site<'_>,
    prd_path: &Path,
    base: &AcceptanceContract,
    owed: &[AcceptanceCriterion],
    model: &str,
    staged: &mut Staged,
    probe: &HostProbe,
) -> WorkflowResult<Result<String, String>> {
    let client = site.llm.expect("the author client was checked");
    let Some(runs) = probe.take_baseline_runs() else {
        return Ok(Err(
            "no pre-implementation tree is known to prove staged checks".into(),
        ));
    };
    let baseline = runs.commit;
    let prd_text = match std::fs::read_to_string(prd_path) {
        Ok(text) => text,
        Err(error) => return Ok(Err(format!("reading the staged checks' PRD: {error}"))),
    };
    for obligation in owed {
        let id = &obligation.id;
        let Some(entry) = staged.entries.get(id).cloned() else {
            continue;
        };
        if entry.id != *id
            || entry.criterion != obligation.criterion
            || entry.covers != obligation.covers
            || entry.judgment.verdict != JudgeDecision::Accepted
        {
            staged.reject(
                id,
                "the staged entry does not match the owed check or was not accepted",
            );
        } else if !staged.proven(id, &baseline, client, model) {
            poll_v2_run_control(site.store, site.run_id, site.call_id)?;
            let contract = working(base, owed, staged);
            let ids = BTreeSet::from([id.clone()]);
            let findings = probe.script_defects(&contract, &ids).await;
            let unproven = probe.take_unproven();
            if !unproven.is_empty() {
                return Ok(Err(HostUnproven(unproven).to_string()));
            }
            let runs = probe
                .take_baseline_runs()
                .expect("the author probe has a baseline");
            let cannot_pass = if findings.is_empty() {
                match cannot_pass_findings(client, model, &contract, &ids, &runs, &prd_text).await {
                    Ok(findings) => findings,
                    Err(error) => return Ok(Err(format!("{error:#}"))),
                }
            } else {
                BTreeMap::new()
            };
            if let Some(why) = findings.get(id).or_else(|| cannot_pass.get(id)) {
                staged.reject(id, why);
            } else {
                staged.accept(entry, &baseline, client, model);
            }
        } else {
            continue;
        }
        if let Err(why) = staged.save(site.run_dir) {
            return Ok(Err(why));
        }
    }
    Ok(Ok(baseline))
}
