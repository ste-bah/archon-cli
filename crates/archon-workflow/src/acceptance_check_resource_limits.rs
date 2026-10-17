use super::CommandEnvironment;
use std::path::Path;

impl CommandEnvironment {
    /// Apply the Bash tool's archon-owned resource defaults, while allowing
    /// the launched command to override them in its own shell.
    pub fn with_workflow_resource_limits(
        mut self,
        limits: &archon_tools::workflow_resource_env::CargoResourceLimits,
        command: &str,
        repository_root: &Path,
    ) -> Self {
        let mut values: Vec<_> = self.variables.into_iter().collect();
        archon_tools::workflow_resource_env::apply_workflow_resource_defaults(
            &mut values,
            command,
            repository_root,
            limits,
        );
        self.variables = values.into_iter().collect();
        self
    }
}
