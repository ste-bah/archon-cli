//! Commands authorized by the frozen task declarations.
use crate::task_universe::WorkflowV2TaskUniverse;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default)]
pub(crate) struct FrozenCommands {
    commands: Option<BTreeSet<String>>,
}
impl FrozenCommands {
    pub(crate) fn from_universe(universe: Option<&WorkflowV2TaskUniverse>) -> Self {
        if universe.is_none() {
            tracing::warn!(reason = %CommandNotRun::NoUniverse, "command execution is unavailable");
        }
        Self {
            commands: universe.map(|universe| {
                universe
                    .tasks
                    .iter()
                    .flat_map(|task| task.declared_focused_test_commands())
                    .collect()
            }),
        }
    }
    pub(crate) fn from_commands(commands: &[String]) -> Self {
        Self {
            commands: Some(commands.iter().cloned().collect()),
        }
    }
    pub(crate) fn command(&self, command: &str) -> Result<&str, CommandNotRun> {
        let commands = self.commands.as_ref().ok_or(CommandNotRun::NoUniverse)?;
        commands
            .get(command)
            .or_else(|| commands.iter().find(|frozen| equivalent(frozen, command)))
            .map(String::as_str)
            .ok_or(CommandNotRun::NotFrozen)
    }
    pub(crate) fn check(&self, command: &str) -> Result<(), CommandNotRun> {
        self.command(command).map(|_| ())
    }
}
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub(crate) enum CommandNotRun {
    #[error("no frozen command set: run metadata has no universe")]
    NoUniverse,
    #[error("the command is absent from the frozen command set")]
    NotFrozen,
}
fn equivalent(left: &str, right: &str) -> bool {
    left.split_whitespace().eq(right.split_whitespace())
}
pub(crate) fn host_runnable(command: &str, frozen: &FrozenCommands) -> bool {
    match frozen.check(command) {
        Ok(()) => true,
        Err(reason) => {
            tracing::debug!(%command, %reason, "command was not run");
            false
        }
    }
}

#[cfg(test)]
#[path = "test_baseline_frozen_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "test_baseline_frozen_proof_tests.rs"]
mod proof_tests;
