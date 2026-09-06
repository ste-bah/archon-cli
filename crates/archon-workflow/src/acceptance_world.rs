//! Frozen command selection. A caller carries identities, never substitute text.
use crate::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, JudgeDecision, TrustedCwd, acceptance_policy_findings,
    content_digest, validate_acceptance_structure,
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
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn cwd(&self) -> TrustedCwd {
        self.cwd
    }
}

/// Select from an already integrity-validated bundle. The execution boundary
/// must revalidate the bundle against the launch pin before calling this function.
pub fn resolve_command(
    contract: &AcceptanceContract,
    chain_digest: &str,
    reference: &FrozenCommandRef,
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
    let prefix = format!("{}.", entry.id);
    if acceptance_policy_findings(contract)
        .iter()
        .any(|finding| finding.field.starts_with(&prefix))
    {
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
