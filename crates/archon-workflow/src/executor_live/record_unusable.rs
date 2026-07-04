use super::*;

impl WorkflowExecutor {
    pub(super) fn record_unusable_output(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        output: &StageRunOutput,
        err: &WorkflowError,
    ) -> WorkflowResult<()> {
        let artifact = persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            &output.extension,
            output.body.clone(),
            false,
        )?;
        persistence::record_agent_output(
            &self.store,
            &run.id,
            &stage.id,
            &stage.id,
            Some(output),
            Some(&artifact),
            false,
            Some(&err.to_string()),
        )
    }
}
