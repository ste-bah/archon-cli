use std::collections::{BTreeMap, BTreeSet};

use super::GUARD_STATUS;
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Markers {
    nonce: String,
}

impl Markers {
    /// The markers a saved earlier attempt used (Issue 255): the same
    /// mutated check, and so its saved verdict, is met again on a retry.
    pub(crate) fn with_nonce(nonce: &str) -> Option<Self> {
        (nonce.len() == 32 && nonce.bytes().all(|b| b.is_ascii_hexdigit())).then(|| Self {
            nonce: nonce.to_string(),
        })
    }

    pub(crate) fn nonce(&self) -> &str {
        &self.nonce
    }

    pub(crate) fn new() -> Self {
        Self {
            nonce: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    pub(crate) fn tag(&self) -> String {
        format!("archon-mutated-{}", &self.nonce[..12])
    }

    pub(crate) fn moved_prefix(&self) -> String {
        format!("archon-mutation-moved-{}: ", self.nonce)
    }

    pub(crate) fn move_state_prefix(&self) -> String {
        format!("archon-mutation-move-state-{}: ", self.nonce)
    }

    pub(crate) fn generated_prefix(&self) -> String {
        format!("archon-mutation-generated-{}: ", self.nonce)
    }

    pub(crate) fn recreated_prefix(&self) -> String {
        format!("archon-mutation-recreated-{}: ", self.nonce)
    }

    #[cfg(test)]
    pub(crate) fn restored(&self) -> String {
        format!("archon-mutation-restored-{}", self.nonce)
    }

    pub(crate) fn guard(&self) -> String {
        format!(
            "archon-mutation-guard-{}: refusing to move inputs in a live root",
            self.nonce
        )
    }

    pub(crate) fn moved(&self, result: &CheckResult, inputs: &[String]) -> Vec<String> {
        let prefix = self.moved_prefix();
        String::from_utf8_lossy(&result.stderr)
            .lines()
            .filter_map(|line| line.strip_prefix(prefix.as_str()))
            .filter_map(|index| {
                index
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| inputs.get(i).cloned())
            })
            .collect()
    }

    pub(crate) fn move_states(
        &self,
        result: &CheckResult,
        inputs: &[String],
    ) -> BTreeMap<String, String> {
        let prefix = self.move_state_prefix();
        String::from_utf8_lossy(&result.stderr)
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix(&prefix)?;
                let (index, state) = rest.split_once(':')?;
                Some((
                    inputs.get(index.parse::<usize>().ok()?)?.clone(),
                    state.to_string(),
                ))
            })
            .collect()
    }

    pub(crate) fn generated(&self, result: &CheckResult, inputs: &[String]) -> BTreeSet<String> {
        let prefix = self.generated_prefix();
        String::from_utf8_lossy(&result.stderr)
            .lines()
            .filter_map(|line| line.strip_prefix(prefix.as_str()))
            .filter_map(|index| {
                index
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| inputs.get(i).cloned())
            })
            .collect()
    }

    pub(crate) fn recreated(&self, result: &CheckResult, inputs: &[String]) -> BTreeSet<String> {
        let prefix = self.recreated_prefix();
        String::from_utf8_lossy(&result.stderr)
            .lines()
            .filter_map(|line| line.strip_prefix(prefix.as_str()))
            .filter_map(|index| {
                index
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| inputs.get(i).cloned())
            })
            .collect()
    }

    pub(crate) fn states(
        &self,
        result: &CheckResult,
        inputs: &[String],
    ) -> BTreeMap<String, String> {
        let prefix = format!("archon-mutation-state-{}: ", self.nonce);
        String::from_utf8_lossy(&result.stdout)
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix(&prefix)?;
                let (index, state) = rest.split_once(':')?;
                let path = inputs.get(index.parse::<usize>().ok()?)?;
                Some((path.clone(), state.to_string()))
            })
            .collect()
    }

    pub(crate) fn status(&self, result: &CheckResult) -> Option<String> {
        let prefix = format!("archon-mutation-status-{}: ", self.nonce);
        String::from_utf8_lossy(&result.stdout)
            .lines()
            .find_map(|line| line.strip_prefix(&prefix).map(str::to_string))
    }

    pub(crate) fn observed_signal(&self, result: &CheckResult) -> Option<String> {
        let status = self.status(result)?;
        let signal = status.split(':').nth(3)?;
        (signal != "none").then(|| signal.to_string())
    }

    #[cfg(test)]
    pub(crate) fn restored_all(&self, result: &CheckResult) -> bool {
        let restored = self.restored();
        String::from_utf8_lossy(&result.stdout)
            .lines()
            .any(|line| line == restored)
    }

    pub(crate) fn guarded(&self, result: &CheckResult) -> bool {
        result.exit_code == Some(GUARD_STATUS)
            && String::from_utf8_lossy(&result.stderr).contains(&self.guard())
    }
}
