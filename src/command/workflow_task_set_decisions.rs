use super::*;
use archon_workflow::requirement_state::DecisionOutcome;
impl PreparedSkeletonFreeze {
    pub(crate) fn task_root(&self) -> &Path {
        &self.tasks_root
    }
    pub(crate) fn classification(
        &self,
    ) -> Option<&archon_workflow::v2::requirement_classes::RequirementClassification> {
        self.lock.classification.as_ref()
    }
    pub(crate) fn with_decisions(
        mut self,
        outcome: &DecisionOutcome,
        extra_findings: Vec<GateFinding>,
    ) -> Result<Self> {
        let mut skeleton: TaskSkeleton = serde_json::from_slice(&self.skeleton_bytes)?;
        outcome.apply_to(&mut skeleton);
        self.findings.extend(extra_findings);
        let stamp = (self.stamp_fn)(self.lock.gate.mode, &self.findings);
        self.skeleton_bytes = serde_json::to_vec_pretty(&skeleton)?;
        self.lock.digest = content_digest(&self.skeleton_bytes);
        self.lock.gate = stamp.clone();
        self.pin.skeleton_digest = Some(self.lock.digest.clone());
        self.pin.skeleton_gate = Some(stamp);
        self.result.skeleton_digest.clone_from(&self.lock.digest);
        Ok(self)
    }
}
