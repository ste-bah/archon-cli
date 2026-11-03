use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::obligation_ids::acceptance_ids;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, content_digest,
};

pub(super) fn read_json<T: for<'de> serde::Deserialize<'de>>(
    path: &Path,
    label: &str,
    freeze: &str,
) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "required {label} {} could not be read: {error}; run workflow {freeze}",
            shown(path)
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "{label} {} is malformed: {error}; re-run workflow {freeze}",
            shown(path)
        )
    })
}

pub(super) fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// A path as the lint report stores it: `/`-separated (Issue 438).
pub(super) fn shown(path: &Path) -> String {
    archon_shell::paths::stored_text(path)
}

pub(super) struct PrdIds {
    pub(super) acceptance: BTreeSet<String>,
    pub(super) implements: BTreeSet<String>,
    pub(super) constraints: BTreeSet<String>,
}

pub(super) fn validate_prd_identity(cwd: &Path, tasks_root: &Path) -> Result<PrdIds, String> {
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract: AcceptanceContract =
        read_json(&contract_path, "acceptance contract", "freeze-acceptance")?;
    let prd_path = absolute(cwd, Path::new(&contract.prd.path));
    let bytes = std::fs::read(&prd_path).map_err(|error| {
        format!(
            "PRD {} could not be read: {error}; restore it or re-run `workflow freeze-acceptance`",
            shown(&prd_path)
        )
    })?;
    let actual = content_digest(&bytes);
    if actual != contract.prd.digest {
        return Err(format!(
            "PRD digest mismatch for {}: expected {}, actual {}; restore the frozen PRD or re-run `workflow freeze-acceptance`",
            shown(&prd_path),
            contract.prd.digest,
            actual
        ));
    }
    let text = String::from_utf8(bytes).map_err(|error| {
        format!(
            "PRD {} is not UTF-8: {error}; restore UTF-8 content and re-run `workflow freeze-acceptance`",
            shown(&prd_path)
        )
    })?;
    let constraints = archon_workflow::obligation_ids::constraint_ids(&text);
    let mut implements = archon_workflow::obligation_ids::obligation_ids(&text);
    implements.extend(constraints.iter().cloned());
    Ok(PrdIds {
        acceptance: acceptance_ids(&text),
        implements,
        constraints,
    })
}
