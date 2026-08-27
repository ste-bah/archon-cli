//! Operational input and predecessor-chain preflight for topology lint.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use super::LintSource;

pub(super) fn operational_input(cwd: &Path, source: &LintSource) -> Result<()> {
    let paths = match source {
        LintSource::TaskFile(path) => vec![super::absolute(cwd, path)],
        LintSource::Tasks(path) => {
            let root = super::absolute(cwd, path);
            archon_workflow::task_universe::task_files_under(&root).map_err(|error| {
                anyhow!(
                    "task directory {} could not be enumerated: {error}; restore it and retry",
                    root.display()
                )
            })?
        }
        LintSource::Spec(_) | LintSource::Graph(_) => return Ok(()),
    };
    if paths.is_empty() {
        return Err(anyhow!(
            "lint examined zero TASK files; add at least one TASK-*.md file before retrying"
        ));
    }
    for path in &paths {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading TASK file {}", path.display()))?;
        archon_workflow::task_universe::parsing::parse_task_file(path, &raw).map_err(|error| {
            anyhow!(
                "{} is malformed: {error}; make the parser-required edit and retry",
                path.display()
            )
        })?;
    }
    if let LintSource::TaskFile(path) = source {
        task_file_freeze(cwd, &super::absolute(cwd, path))?;
    }
    Ok(())
}

pub(super) fn task_file_freeze(cwd: &Path, path: &Path) -> Result<()> {
    use archon_workflow::obligation_ids::acceptance_ids;
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, AcceptancePin, content_digest,
        validate_acceptance_bundle,
    };
    use archon_workflow::task_skeleton::validate_full_chain;

    let root = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent task directory", path.display()))?;
    if super::task_set::freeze_chain_is_absent(cwd, root) {
        return Ok(());
    }

    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(cwd, root);
    let pin: AcceptancePin = serde_json::from_slice(
        &std::fs::read(&pin_path)
            .with_context(|| format!("reading acceptance pin {}", pin_path.display()))?,
    )
    .with_context(|| {
        format!(
            "acceptance pin {} is malformed or unstamped",
            pin_path.display()
        )
    })?;
    let contract_path = root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract: AcceptanceContract = serde_json::from_slice(
        &std::fs::read(&contract_path)
            .with_context(|| format!("reading acceptance contract {}", contract_path.display()))?,
    )
    .with_context(|| {
        format!(
            "acceptance contract {} is malformed",
            contract_path.display()
        )
    })?;
    let prd_path = {
        let path = PathBuf::from(&contract.prd.path);
        if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        }
    };
    let prd = std::fs::read(&prd_path)
        .with_context(|| format!("reading frozen PRD {}", prd_path.display()))?;
    if content_digest(&prd) != contract.prd.digest {
        return Err(anyhow!(
            "PRD digest mismatch for {}; restore it or re-run workflow freeze-acceptance",
            prd_path.display()
        ));
    }
    let expected = acceptance_ids(std::str::from_utf8(&prd).context("frozen PRD is not UTF-8")?);
    validate_acceptance_bundle(root, Some(&pin), &expected)
        .map_err(|error| anyhow!(error.to_string()))?;

    let skeleton_path = root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    let skeleton_lock_path = root.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE);
    let skeleton_state = (
        skeleton_path.exists(),
        skeleton_lock_path.exists(),
        pin.skeleton_digest.is_some() || pin.skeleton_gate.is_some(),
    );
    match skeleton_state {
        (false, false, false) => Ok(()),
        (true, true, true) => {
            validate_full_chain(root, &pin).map_err(|error| anyhow!(error.to_string()))?;
            Ok(())
        }
        (file, lock, pin) => Err(anyhow!(
            "partial skeleton freeze beside {}: file={}, lock={}, pin={}; restore a matching frozen triple or remove all successor artifacts",
            path.display(),
            file,
            lock,
            pin
        )),
    }
}
