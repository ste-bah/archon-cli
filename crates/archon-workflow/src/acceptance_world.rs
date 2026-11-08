//! Frozen command selection. A caller carries identities, never substitute text.
use crate::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, JudgeDecision, TrustedCwd,
    acceptance_entry_policy_findings_with_bindings, content_digest, validate_acceptance_structure,
};
use crate::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcceptanceCommandKind {
    Command,
    NestedVerifier,
    ResidualFailClosed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenCommandRef {
    pub acceptance_id: String,
    pub kind: AcceptanceCommandKind,
    pub chain_digest: String,
    pub command_digest: String,
}

#[derive(Debug)]
pub struct AuthorizedCommand {
    bytes: Vec<u8>,
    cwd: TrustedCwd,
}
impl AuthorizedCommand {
    /// A command for runner tests, which drive `run_at` without a contract.
    #[cfg(test)]
    pub(crate) fn for_test(text: &str, cwd: TrustedCwd) -> Self {
        Self {
            bytes: text.as_bytes().to_vec(),
            cwd,
        }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn cwd(&self) -> TrustedCwd {
        self.cwd
    }
}

/// Policy defects that prevent the host from running this check. Judgment
/// contradictions induced by repairable expansion findings do not block execution.
pub fn execution_policy_defects(
    entry: &crate::task_set_contract::AcceptanceCriterion,
    bound: &[&str],
) -> Vec<crate::task_set_contract::AcceptancePolicyFinding> {
    acceptance_entry_policy_findings_with_bindings(entry, bound)
        .into_iter()
        .filter(|finding| finding.field == format!("{}.check", entry.id))
        .filter(|finding| {
            finding
                .identity
                .as_ref()
                .is_none_or(|identity| identity.code != "unset_shell_variable")
        })
        .collect()
}

/// Select from an already integrity-validated bundle. The execution boundary
/// must revalidate the bundle against the launch pin before calling this function.
pub fn resolve_command(
    contract: &AcceptanceContract,
    chain_digest: &str,
    reference: &FrozenCommandRef,
) -> WorkflowResult<AuthorizedCommand> {
    resolve_command_with_bindings(contract, chain_digest, reference, &[])
}

/// Resolve using the bound names of the run's environment snapshot.
pub fn resolve_command_with_bindings(
    contract: &AcceptanceContract,
    chain_digest: &str,
    reference: &FrozenCommandRef,
    bound: &[&str],
) -> WorkflowResult<AuthorizedCommand> {
    let reject = |reason: &str| WorkflowError::ArtifactInvalid(reason.to_string());
    if reference.chain_digest != chain_digest {
        return Err(reject("acceptance command chain identity changed"));
    }
    let expected = contract
        .acceptance
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    validate_acceptance_structure(contract, &expected, true)
        .map_err(|error| reject(&error.to_string()))?;
    let entry = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .find(|entry| entry.id == reference.acceptance_id)
        .ok_or_else(|| reject("acceptance command id is not in the pinned contract"))?;
    if entry.judgment.verdict != JudgeDecision::Accepted {
        return Err(reject("acceptance command was not accepted by the judge"));
    }
    // Expansion findings are handled by in-round author repair. Runtime
    // execution still classifies any fatal diagnostic before counting a pass.
    if !execution_policy_defects(entry, bound).is_empty() {
        return Err(reject("acceptance command carries a host policy defect"));
    }
    let (command, cwd) = match (&entry.check, reference.kind) {
        (AcceptanceCheck::Command { command, cwd }, AcceptanceCommandKind::Command) => {
            (command.as_str(), *cwd)
        }
        (AcceptanceCheck::Floor { contract }, AcceptanceCommandKind::NestedVerifier) => (
            contract
                .typed_verifier_command
                .as_deref()
                .ok_or_else(|| reject("floor has no pinned verifier command"))?,
            TrustedCwd::ProjectRoot,
        ),
        (_, AcceptanceCommandKind::ResidualFailClosed) => {
            return Err(reject(
                "residual fail_closed_check has no pinned judged binding; refusing native execution",
            ));
        }
        _ => {
            return Err(reject(
                "acceptance command kind does not match pinned check",
            ));
        }
    };
    if content_digest(command.as_bytes()) != reference.command_digest {
        return Err(reject("acceptance command digest changed"));
    }
    Ok(AuthorizedCommand {
        bytes: command.as_bytes().to_vec(),
        cwd,
    })
}
