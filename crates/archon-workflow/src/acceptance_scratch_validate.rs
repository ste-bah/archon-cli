use super::*;

impl ScratchPolicy {
    pub fn validate(&self) -> WorkflowResult<()> {
        for root in [
            &self.repository,
            &self.project,
            &self.task_root,
            &self.scratch_parent,
        ]
        .into_iter()
        .chain(&self.build_cache)
        {
            if !root.is_absolute() || root.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err(invalid(
                    "scratch policy roots must be absolute and normalized",
                ));
            }
        }
        if self.timeout_secs == 0 || self.output_bytes == 0 || self.scratch_bytes == 0 {
            return Err(invalid("scratch limits must be positive"));
        }
        for input in self
            .project_inputs
            .iter()
            .chain(&self.project_input_excludes)
        {
            if !relative(input) || input.components().any(|c| c.as_os_str() == ".git") {
                return Err(invalid(
                    "project input must be a normalized relative path without .git",
                ));
            }
        }
        crate::acceptance_check_environment::validate_operator_bindings(
            &self.toolchain_path,
            &self.environment,
            &self.environment_allowlist,
        )
        .map_err(invalid)?;
        Ok(())
    }
}
