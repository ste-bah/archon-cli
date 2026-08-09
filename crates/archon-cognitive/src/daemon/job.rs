use archon_policy::CognitivePolicy;
use cozo::DbInstance;
use serde::{Deserialize, Serialize};

use crate::{CognitiveError, CognitiveTick};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonJobReport {
    pub name: String,
    pub ok: bool,
    pub summary: String,
}

pub trait DaemonJob {
    fn name(&self) -> &'static str;
    fn run(&mut self) -> Result<DaemonJobReport, CognitiveError>;
}

pub struct CognitiveTickJob<'a> {
    db: &'a DbInstance,
    policy: CognitivePolicy,
    ledger_dir: std::path::PathBuf,
}

impl<'a> CognitiveTickJob<'a> {
    pub fn new(
        db: &'a DbInstance,
        policy: CognitivePolicy,
        ledger_dir: impl AsRef<std::path::Path>,
    ) -> Self {
        Self {
            db,
            policy,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
        }
    }
}

impl DaemonJob for CognitiveTickJob<'_> {
    fn name(&self) -> &'static str {
        "cognitive_tick"
    }

    fn run(&mut self) -> Result<DaemonJobReport, CognitiveError> {
        let report =
            CognitiveTick::new(self.db, Some(self.policy.clone()), &self.ledger_dir)?.tick()?;
        let ok = report.errors.is_empty();
        Ok(DaemonJobReport {
            name: self.name().into(),
            ok,
            summary: format!(
                "ticks proposals={} generated={} replayed={} self_model={} errors={}",
                report.proposals_evaluated,
                report.proposals_generated,
                measured(report.dead_letters_replayed),
                measured(report.self_model_updated),
                report.errors.len()
            ),
        })
    }
}

/// Render a step that could not run as "not measured" rather than as a zero a
/// reader would take for a result.
fn measured<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "not_measured".to_string(), |value| value.to_string())
}
