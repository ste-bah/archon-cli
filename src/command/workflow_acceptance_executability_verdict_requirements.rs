//! Covered requirement evidence for an absent subcommand.
use super::*;
impl Context {
    pub(crate) fn original_usage_error(&self, result: &CheckResult, command: &str) -> bool {
        let root = std::env::current_dir().unwrap_or_default();
        let (repository, project) = self
            .roots
            .as_ref()
            .map_or((&root, &root), |(repo, project)| (repo, project));
        let runs = super::super::BaselineRuns {
            reuse: BTreeMap::new(),
            commit: String::new(),
            failures: [(
                result.acceptance_id.clone(),
                super::super::hermetic::CheckEvidence {
                    result: result.clone(),
                    copy_layout: None,
                },
            )]
            .into(),
            environment: self.environment.clone(),
            forwarded: Vec::new(),
        };
        super::super::super::passability::original_usage_error(
            &runs,
            &result.acceptance_id,
            command,
            repository,
            project,
        )
    }

    pub(crate) fn with_requirements(
        mut self,
        probe: &super::super::HostProbe,
        contract: &AcceptanceContract,
    ) -> Self {
        self.roots = Some((probe.repository.clone(), probe.project.clone()));
        let text = [&probe.project, &probe.repository]
            .into_iter()
            .find_map(|root| std::fs::read_to_string(root.join(&contract.prd.path)).ok())
            .unwrap_or_default();
        let requirements =
            archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(&text);
        self.requirements = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .map(|entry| {
                (
                    entry.id.clone(),
                    entry
                        .covers
                        .iter()
                        .filter_map(|id| requirements.get(id).cloned())
                        .collect(),
                )
            })
            .collect();
        self
    }
}

/// Reuse the unchanged program-absence rules with the actual launch variables.
pub(crate) fn missing_site_program(
    command: &str,
    result: &CheckResult,
    environment: &BTreeMap<String, String>,
) -> Option<String> {
    let at = Context::at(environment.clone(), Vec::new());
    let (_, cause) = no_verdict_cause(command, result, &at)?;
    if !matches!(
        cause,
        UnprovenCause::ProgramAbsent | UnprovenCause::SiteLacks
    ) {
        return None;
    }
    let output = output(result);
    let program = missing_program(&simple_commands(command), &output, &at)?;
    let condition = if mutation::reported_permission_denied(&output, &program) {
        "not executable"
    } else {
        "not found"
    };
    Some(format!("{program} {condition}"))
}
