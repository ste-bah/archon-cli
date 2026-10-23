use super::*;

/// Re-prepare a freeze from a contract whose checks were re-judged in place
/// and proven on `baseline_commit`.
pub(crate) fn prepare_from_judged(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    contract: &AcceptanceContract,
    baseline_commit: Option<String>,
    live_contract_digest: Option<String>,
) -> Result<PreparedAcceptanceFreeze> {
    let (prd, _, _) = validate_prd_input(prd_path)?;
    let prd_text = String::from_utf8(prd).context("PRD is not UTF-8")?;
    let candidate_bytes = serde_json::to_vec_pretty(contract)?;
    let candidate_digest = content_digest(&candidate_bytes);
    let mut prepared = finish_acceptance(
        project_root,
        tasks_root,
        prd_path,
        &prd_text,
        freeze_mode(mode)?,
        contract,
        Vec::new(),
        baseline_commit,
        candidate_digest.clone(),
    )?;
    prepared.live_contract_digest = live_contract_digest.clone();
    prepared.result.live_contract_digest = live_contract_digest;
    Ok(prepared)
}

/// Capture and validate the live contract snapshot before work that may
/// re-author it. The returned digest must travel through preparation to the
/// publication compare-and-swap.
pub(crate) fn capture_live_contract_digest(tasks_root: &Path) -> Result<Option<String>> {
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    match std::fs::read(&contract_path) {
        Ok(live_bytes) => {
            serde_json::from_slice::<AcceptanceContract>(&live_bytes)
                .with_context(|| format!("parsing {}", contract_path.display()))?;
            Ok(Some(content_digest(&live_bytes)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", contract_path.display())),
    }
}
