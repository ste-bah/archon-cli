//! Fault classes are selected while typed causes are still available.
use archon_workflow::{GateOperationalError, WorkflowError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Class {
    Transient,
    Permanent,
}
impl Class {
    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::Transient => "transient_host_fault",
            Self::Permanent => "permanent_host_fault",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Fault {
    pub(crate) class: Class,
    pub(crate) text: String,
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}
impl std::error::Error for Fault {}

/// Sealing refused the child's evidence; a permanent refusal remains an error.
#[derive(Debug)]
pub(crate) struct EvidenceRefused(pub(crate) String);
impl std::fmt::Display for EvidenceRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for EvidenceRefused {}
pub(crate) fn evidence_refused(error: &WorkflowError) -> bool {
    matches!(error, WorkflowError::Port(source) if source.is::<EvidenceRefused>())
}

pub(crate) fn transient(text: String) -> anyhow::Error {
    Fault {
        class: Class::Transient,
        text,
    }
    .into()
}

/// Transparent workflow variants forward `source()` past their boxed root.
/// Inspect that root as well, so a positive typed cause is never discarded.
fn causes(error: &anyhow::Error) -> Vec<&(dyn std::error::Error + 'static)> {
    let mut causes = Vec::new();
    let mut next: Option<&(dyn std::error::Error + 'static)> = Some(error.as_ref());
    while let Some(cause) = next {
        causes.push(cause);
        next = match cause.downcast_ref::<WorkflowError>() {
            Some(WorkflowError::Port(source)) => Some(source.as_ref()),
            Some(WorkflowError::Json(source)) => Some(source),
            Some(WorkflowError::Yaml(source)) => Some(source),
            Some(WorkflowError::TomlSerialize(source)) => Some(source),
            Some(WorkflowError::TomlDeserialize(source)) => Some(source),
            _ => cause
                .downcast_ref::<std::io::Error>()
                .and_then(|error| {
                    error
                        .get_ref()
                        .map(|source| source as &(dyn std::error::Error + 'static))
                })
                .or_else(|| cause.source()),
        };
    }
    causes
}

pub(crate) fn classify(error: &anyhow::Error) -> Class {
    if let Some(fault) = error.downcast_ref::<Fault>() {
        return fault.class;
    }
    for cause in causes(error) {
        if let Some(fault) = cause.downcast_ref::<Fault>() {
            return fault.class;
        }
    }
    for cause in causes(error) {
        if let Some(contract) =
            cause.downcast_ref::<archon_workflow::task_set_contract::TaskSetContractError>()
        {
            if let Some((kind, raw)) = contract.io_cause {
                let error = raw
                    .map(std::io::Error::from_raw_os_error)
                    .unwrap_or_else(|| std::io::Error::from(kind));
                return io_class(&error);
            }
        }
    }
    for cause in causes(error) {
        if let Some(skeleton) =
            cause.downcast_ref::<archon_workflow::task_skeleton::TaskSkeletonError>()
        {
            if let Some((kind, raw)) = skeleton.io_cause {
                let error = raw
                    .map(std::io::Error::from_raw_os_error)
                    .unwrap_or_else(|| std::io::Error::from(kind));
                return io_class(&error);
            }
        }
    }
    for cause in causes(error) {
        if let Some(json) = cause.downcast_ref::<serde_json::Error>() {
            if let Some(kind) = json.io_error_kind() {
                return io_class(&std::io::Error::from(kind));
            }
        }
    }
    // A validation/corruption producer's type overrides incidental nested I/O.
    for cause in causes(error) {
        if matches!(
            cause.downcast_ref::<WorkflowError>(),
            Some(
                WorkflowError::ControlPaused(_)
                    | WorkflowError::ControlCancelled(_)
                    | WorkflowError::StateCorrupt(_)
                    | WorkflowError::SpecInvalid(_)
                    | WorkflowError::ArtifactInvalid(_)
                    | WorkflowError::Json(_)
                    | WorkflowError::Yaml(_)
                    | WorkflowError::TomlDeserialize(_)
                    | WorkflowError::TaskBodyInvalid(_)
                    | WorkflowError::PolicyDenied(_)
            )
        ) || cause.is::<archon_workflow::task_set_contract::TaskSetContractError>()
            || cause.is::<archon_workflow::task_skeleton::TaskSkeletonError>()
            || cause.is::<serde_json::Error>()
            || cause.is::<toml::de::Error>()
        {
            return Class::Permanent;
        }
    }
    for cause in causes(error) {
        if cause.is::<archon_llm::transport_idle::TransportIdle>()
            || cause.is::<super::topology_lint::LintIncomplete>()
        {
            return Class::Transient;
        }
        if matches!(
            cause.downcast_ref::<WorkflowError>(),
            Some(WorkflowError::HostCallTimeout(_))
        ) {
            return Class::Transient;
        }
        if let Some(error) = cause.downcast_ref::<archon_llm::provider::LlmError>() {
            use archon_llm::provider::LlmError;
            return match error {
                // An untyped HTTP diagnostic also represents rejected requests.
                // Only a typed availability cause authorizes another dispatch.
                LlmError::RateLimited { .. }
                | LlmError::Overloaded { .. }
                | LlmError::Transport(_)
                | LlmError::Aborted => Class::Transient,
                LlmError::Server { status, .. }
                    if *status >= 500 || matches!(*status, 408 | 429) =>
                {
                    Class::Transient
                }
                _ => Class::Permanent,
            };
        }
    }
    for cause in causes(error) {
        if let Some(incomplete) =
            cause.downcast_ref::<super::workflow_task_set::judge::JudgeIncomplete>()
        {
            return incomplete.fault_class;
        }
        if let Some(incomplete) =
            cause.downcast_ref::<super::workflow_freeze_budget::FreezeIncomplete>()
        {
            return incomplete.fault_class();
        }
    }
    for cause in causes(error) {
        if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            // `Other` may only wrap a more precise typed cause.
            if error.kind() == std::io::ErrorKind::Other && error.get_ref().is_some() {
                continue;
            }
            return io_class(error);
        }
    }
    for cause in causes(error) {
        if let Some(unsettled) = cause.downcast_ref::<super::workflow_task_set::UnsettledPublish>()
        {
            return unsettled.fault_class();
        }
    }
    // Unknown producers retain their own diagnostic; they never silently loop.
    Class::Permanent
}

pub(crate) fn diagnostic(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    if classify(error) == Class::Transient {
        return text;
    }
    for cause in causes(error) {
        let kind = if let Some(source) = cause.downcast_ref::<std::io::Error>() {
            if source.kind() == std::io::ErrorKind::Other && source.get_ref().is_some() {
                continue;
            }
            Some(source.kind())
        } else if let Some(contract) =
            cause.downcast_ref::<archon_workflow::task_set_contract::TaskSetContractError>()
        {
            contract.io_cause.map(|(kind, _)| kind)
        } else if let Some(skeleton) =
            cause.downcast_ref::<archon_workflow::task_skeleton::TaskSkeletonError>()
        {
            skeleton.io_cause.map(|(kind, _)| kind)
        } else {
            None
        };
        let Some(kind) = kind else {
            continue;
        };
        let remedy = match kind {
            std::io::ErrorKind::StorageFull => {
                "free disk space on the volume containing the named path"
            }
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
                "restore access permission on the named path or program"
            }
            std::io::ErrorKind::NotFound => "restore the named required file or directory",
            std::io::ErrorKind::AlreadyExists
            | std::io::ErrorKind::NotADirectory
            | std::io::ErrorKind::IsADirectory => {
                "restore the named file or directory, removing the obstructing entry"
            }
            std::io::ErrorKind::InvalidData => "restore valid bytes in the named artifact",
            _ => return text,
        };
        return format!("{text}; remedy: {remedy}");
    }
    text
}

pub(crate) fn envelope(error: &anyhow::Error) -> GateOperationalError {
    GateOperationalError {
        kind: classify(error).kind().into(),
        text: diagnostic(error),
    }
}

/// Only the trusted child's reserved stdout carries a pre-envelope fault.
pub(crate) fn child_failure(error: anyhow::Error) -> anyhow::Error {
    println!(
        "{}",
        serde_json::to_string(&envelope(&error)).expect("fault serialization")
    );
    error
}

pub(crate) fn workflow(error: anyhow::Error, context: String) -> WorkflowError {
    if matches!(
        error.downcast_ref::<WorkflowError>(),
        Some(WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_))
    ) {
        return error
            .downcast::<WorkflowError>()
            .expect("checked control error");
    }
    WorkflowError::port(Fault {
        class: classify(&error),
        text: format!("{context}: {}", diagnostic(&error)),
    })
}

pub(crate) fn finding_class(finding: &archon_workflow::GatePolicyFinding) -> Class {
    match finding
        .deterministic_defect
        .as_ref()
        .map(|defect| defect.code.as_str())
    {
        Some("host_transient" | "host_site_lacks") => Class::Transient,
        _ => Class::Permanent,
    }
}

pub(crate) fn io_class(error: &std::io::Error) -> Class {
    if matches!(
        error.kind(),
        std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::AddrInUse
    ) || (cfg!(unix) && matches!(error.raw_os_error(), Some(5 | 16)))
        || (cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)))
    {
        Class::Transient
    } else {
        Class::Permanent
    }
}
pub(crate) fn is_kind(value: &str) -> bool {
    matches!(
        value,
        "transient" | "permanent" | "transient_host_fault" | "permanent_host_fault"
    )
}
pub(crate) fn is_finding_code(value: &str) -> bool {
    matches!(
        value,
        "host_transient" | "host_permanent" | "host_site_lacks" | "host_program_absent"
    )
}
