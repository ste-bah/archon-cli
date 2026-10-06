//! At run start, warn when the frozen checks run commands the configured
//! toolchain path cannot resolve (Issue 331).
//!
//! A check that runs a program, or a subcommand of a tool (`tool sub`),
//! that its `[workflow.acceptance_execution] toolchain_path` does not
//! provide can never pass there, however good the implementation is. The
//! host says so once, before any stage runs, so an operator sees it before
//! a long run: an event (`toolchain_unresolved`) and a log line. It is a
//! warning only: the run goes on, and nothing is refused or capped. A
//! subcommand is named when the path lacks its `tool-sub` program and the
//! host has it, or the tool lists its commands without it and no program
//! anywhere provides it (Issue 333); the tools that decides are listed on
//! a blocking thread, with the scratch site's own environment.

use std::collections::BTreeMap;

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, JudgeDecision,
};
use archon_workflow::{SharedWorkflowUiSink, WorkflowEventKind, WorkflowEventLog, WorkflowStore};

use crate::command::acceptance_scratch_policy::NativeBinding;
use crate::command::workflow_task_set::executability::{
    CheckSite, executed_text, unresolved_on_path,
};

/// What a run's checks run that the toolchain path cannot resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unresolved {
    pub(crate) toolchain_path: String,
    /// Each accepted check that runs something unresolved, by id.
    pub(crate) checks: BTreeMap<String, Vec<String>>,
}

impl Unresolved {
    pub(crate) fn event_detail(&self) -> serde_json::Value {
        serde_json::json!({
            "event": "toolchain_unresolved",
            "toolchain_path": self.toolchain_path,
            "checks": self.checks,
            "effect": "a warning only: the run goes on, but these checks cannot pass until the toolchain path provides what they run",
        })
    }

    pub(crate) fn summary_line(&self) -> String {
        let checks: Vec<String> = (self.checks.iter())
            .map(|(id, commands)| format!("{id}: {}", commands.join(", ")))
            .collect();
        format!(
            "Warning: {} acceptance check(s) run commands the configured toolchain_path ({}) does not resolve; they cannot pass until it provides them: {}\n",
            self.checks.len(),
            self.toolchain_path,
            checks.join("; ")
        )
    }
}

/// What `contract`'s accepted checks run that the scratch site `at` (of the
/// toolchain path `toolchain_path`) does not resolve; `None` when it
/// resolves everything. Blocking: it may list the tools the checks run.
pub(crate) fn unresolved(
    contract: &AcceptanceContract,
    toolchain_path: &str,
    at: &CheckSite,
) -> Option<Unresolved> {
    let accepted: Vec<(&str, &str)> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .filter_map(|entry| Some((entry.id.as_str(), executed_text(entry)?.1)))
        .collect();
    let texts: Vec<&str> = accepted.iter().map(|(_, text)| *text).collect();
    let checks: BTreeMap<String, Vec<String>> = (accepted.iter())
        .zip(unresolved_on_path(&texts, at))
        .filter(|(_, missing)| !missing.is_empty())
        .map(|((id, _), missing)| (id.to_string(), missing))
        .collect();
    (!checks.is_empty()).then(|| Unresolved {
        toolchain_path: toolchain_path.to_string(),
        checks,
    })
}

/// The run's recorded scratch policy and frozen contract, if it has both.
fn recorded(store: &WorkflowStore, run_id: &str) -> Option<(NativeBinding, AcceptanceContract)> {
    let snapshot = super::super::load_generated_v2_metadata(store, run_id)
        .ok()??
        .observer_snapshot?;
    let binding = (snapshot.native_execution)
        .filter(|value| value.get("policy").is_some())
        .and_then(|value| serde_json::from_value::<NativeBinding>(value).ok())?;
    let contract =
        std::path::Path::new(&snapshot.canonical_task_root_identity).join(ACCEPTANCE_CONTRACT_FILE);
    let contract = serde_json::from_slice(&std::fs::read(contract).ok()?).ok()?;
    Some((binding, contract))
}

/// Warn, once at the start of run `run_id`, about what its checks run that
/// its toolchain path does not resolve. Never fails the run.
pub(crate) async fn warn(store: &WorkflowStore, run_id: &str, ui_sink: &SharedWorkflowUiSink) {
    let Some((binding, contract)) = recorded(store, run_id) else {
        return;
    };
    // The site is read here, then its tools are listed on a blocking
    // thread: a slow one never holds an async worker (Issue 333).
    let host = archon_workflow::acceptance_check_environment::host_environment();
    let at = CheckSite::for_scratch(&binding.policy, &host);
    let path = binding.policy.toolchain_path;
    let found = match tokio::task::spawn_blocking(move || unresolved(&contract, &path, &at)).await {
        Ok(Some(found)) => found,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(run_id, %error, "checking the toolchain path failed");
            return;
        }
    };
    let line = found.summary_line();
    tracing::warn!(run_id, "{}", line.trim_end());
    let emitted = store.next_event_seq(run_id).and_then(|seq| {
        WorkflowEventLog::new(store.clone()).emit(
            run_id,
            seq,
            WorkflowEventKind::Started,
            found.event_detail(),
        )
    });
    if let Err(error) = emitted {
        tracing::warn!(run_id, %error, "recording the toolchain warning failed");
    }
    if let Err(error) = ui_sink
        .emit(archon_workflow::WorkflowUiEvent::Text(line))
        .await
    {
        tracing::warn!(run_id, %error, "reporting the toolchain warning failed");
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_run_start_toolchain_tests.rs"]
mod tests;
