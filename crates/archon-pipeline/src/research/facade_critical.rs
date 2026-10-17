macro_rules! critical_artifact_methods {
    () => {
        fn critical_validator_artifact(&self, agent: &AgentInfo) -> Option<String> {
            (agent.critical && agent.key == final_assembly::VALIDATOR_KEY)
                .then(|| final_assembly::COMBINER_KEY.to_string())
        }

        fn critical_validator_artifact_text(
            &self,
            session: &PipelineSession,
            validator: &AgentInfo,
        ) -> Option<String> {
            self.critical_validator_artifact(validator)?;
            super::draft_artifact::paper_to_validate(session)
        }

        fn build_critical_repair_prompt(
            &self,
            _session: &PipelineSession,
            _validator: &AgentInfo,
            artifact: &str,
            findings: &str,
        ) -> Result<String> {
            Ok(super::draft_artifact::repair_prompt(artifact, findings))
        }

        fn write_failed_critical_draft(
            &self,
            _session: &PipelineSession,
            artifact: &str,
            findings: &str,
            bundle_dir: &std::path::Path,
        ) -> Result<Option<Vec<(String, std::path::PathBuf, String)>>> {
            Ok(Some(super::draft_artifact::write_draft_research_artifacts(
                bundle_dir, artifact, findings,
            )?))
        }
    };
}
