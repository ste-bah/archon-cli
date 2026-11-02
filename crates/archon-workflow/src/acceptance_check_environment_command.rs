//! The verifier command boundary.
use super::*;

impl CommandEnvironment {
    /// Acceptance checks consume a complete launch/site policy: site_variables
    /// installs the configured PATH and site-owned HOME/cache directories;
    /// forwarded_values requires valid, present operator-selected data; proxy
    /// defaults accept only bare addresses. Adding shared ambient defaults here
    /// would replace those bindings or introduce variables outside that recorded
    /// policy. The shared read/spawn boundary still applies to this exact map.
    /// Every child still passes through the process-wide spawn boundary.
    pub fn command(&self, program: impl AsRef<OsStr>) -> std::process::Command {
        archon_shell::execution_environment::command_with_mode(
            program,
            archon_shell::execution_environment::Mode::Exact(&self.variables),
        )
    }

    /// Host-composed macOS write/read profile; no argv-based exemption.
    #[cfg(target_os = "macos")]
    pub fn macos_sandbox_command(&self, program: &OsStr, profile: &str) -> std::process::Command {
        let mut command = archon_shell::read_boundary::macos_command(program, profile);
        archon_shell::execution_environment::apply_mode(
            &mut command,
            archon_shell::execution_environment::Mode::Exact(&self.variables),
        );
        command
    }

    pub fn tokio_command(&self, program: impl AsRef<OsStr>) -> tokio::process::Command {
        tokio::process::Command::from(self.command(program))
    }
}
