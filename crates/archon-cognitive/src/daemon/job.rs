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
}

impl<'a> CognitiveTickJob<'a> {
    pub fn new(db: &'a DbInstance, policy: CognitivePolicy) -> Self {
        Self { db, policy }
    }
}

impl DaemonJob for CognitiveTickJob<'_> {
    fn name(&self) -> &'static str {
        "cognitive_tick"
    }

    fn run(&mut self) -> Result<DaemonJobReport, CognitiveError> {
        let report = CognitiveTick::new(self.db, Some(self.policy.clone()))?.tick()?;
        let ok = report.errors.is_empty();
        Ok(DaemonJobReport {
            name: self.name().into(),
            ok,
            summary: format!(
                "ticks proposals={} generated={} errors={}",
                report.proposals_evaluated,
                report.proposals_generated,
                report.errors.len()
            ),
        })
    }
}
