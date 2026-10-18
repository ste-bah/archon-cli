use super::BashTool;

impl BashTool {
    /// Set the resource defaults for agent-run `cargo` commands without
    /// changing this tool's command classification policy.
    pub fn with_cargo_limits(
        mut self,
        cargo_limits: crate::workflow_resource_env::CargoResourceLimits,
    ) -> Self {
        self.cargo_limits = cargo_limits;
        self
    }
}
