//! The launch guard of the frozen chain: a run is refused while its bound
//! contract carries a check the judge did not accept, and the operator is
//! given the exact per-check repair command.

use super::*;

/// The exact operator command that repairs `ids` in the contract beside
/// `tasks_root`.
pub(crate) fn reauthor_command(
    project_root: &Path,
    tasks_root: &Path,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
) -> String {
    let prd = PathBuf::from(&contract.prd.path);
    let prd = if prd.is_absolute() {
        prd
    } else {
        project_root.join(prd)
    };
    format!(
        "cd {} && archon workflow freeze-acceptance {} --tasks {} --prd {}",
        project_root.display(),
        ids.iter()
            .map(|id| format!("--reauthor {id}"))
            .collect::<Vec<_>>()
            .join(" "),
        tasks_root.display(),
        prd.display()
    )
}

/// Refuse to launch a run bound to a contract carrying any check the judge
/// did not accept (such a check can never pass, so the run could never
/// complete), or bound to a freeze the host cannot read back: an unreadable
/// contract, or a lock whose pin is missing or unreadable. Any publish of the
/// set a crash interrupted is first settled to one whole version (Issue 271).
/// A task set with no contract and no lock launches.
pub(crate) fn refuse_unaccepted_launch(project_root: &Path, tasks_root: &Path) -> Result<()> {
    let (_publish_lock, _) =
        super::super::lock_and_recover(&acceptance_pin_path(project_root, tasks_root), tasks_root)?;
    #[cfg(test)]
    super::super::publish::reader_test_step("launch-read");
    let path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let locked = tasks_root.join(ACCEPTANCE_LOCK_FILE).exists();
    if !path.exists() && !locked {
        return Ok(());
    }
    let contract: AcceptanceContract = std::fs::read(&path)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(anyhow::Error::from))
        .with_context(|| {
            format!(
                "refusing to launch: the bound acceptance contract {} cannot be read; restore it or re-freeze",
                path.display()
            )
        })?;
    if !locked {
        super::super::publish::verify_recovered_chain(
            &acceptance_pin_path(project_root, tasks_root),
            tasks_root,
        )
        .map_err(|reason| {
            anyhow!("refusing to launch: the bound frozen chain is inconsistent: {reason}")
        })?;
    }
    if locked {
        let pin_path = acceptance_pin_path(project_root, tasks_root);
        std::fs::read(&pin_path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| {
                serde_json::from_slice::<AcceptancePin>(&bytes).map_err(anyhow::Error::from)
            })
            .with_context(|| {
                format!(
                    "refusing to launch: {} is frozen but its pin {} cannot be read; restore it or re-freeze",
                    path.display(),
                    pin_path.display()
                )
            })?;
    }
    let bad = non_accepted_ids(&contract);
    if bad.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "refusing to launch: {} is bound to this run and carries {} check(s) the judge did not accept, which can never pass:\n  - {}\nRepair only those entries (the skeleton chain is re-bound in the same step), then launch again:\n  {}",
        path.display(),
        bad.len(),
        reauthor::not_accepted_lines(&contract, &bad).join("\n  - "),
        reauthor_command(project_root, tasks_root, &contract, &bad)
    ))
}
