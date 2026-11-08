//! Keep inherited configuration and filters out of isolated source copies.
use super::*;

pub(super) struct IsolatedGit {
    configuration: PathBuf,
    template: PathBuf,
}
impl IsolatedGit {
    pub(super) fn new(root: &Path) -> Result<Self, Unrun> {
        std::fs::create_dir_all(root)
            .map_err(|error| Unrun::error(error.into(), "creating isolated copy storage".into()))?;
        let configuration = root.join(uuid::Uuid::new_v4().to_string());
        let template = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&configuration, []).map_err(|error| {
            Unrun::error(error.into(), "creating isolated configuration".into())
        })?;
        std::fs::create_dir(&template)
            .map_err(|error| Unrun::error(error.into(), "creating isolated template".into()))?;
        Ok(Self {
            configuration,
            template,
        })
    }
    pub(super) fn template(&self) -> &Path {
        &self.template
    }
    pub(super) fn run(&self, args: &[&std::ffi::OsStr]) -> Result<(), Unrun> {
        let output = self.output(args)?;
        if output.status.success() {
            return Ok(());
        }
        let cause = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(Unrun::unknown(if cause.is_empty() {
            format!("isolated operation exited with {}", output.status)
        } else {
            cause
        }))
    }
    pub(super) fn output(&self, args: &[&std::ffi::OsStr]) -> Result<std::process::Output, Unrun> {
        let mut command = archon_shell::spawn::command("git");
        for (name, _) in std::env::vars_os() {
            if name
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(name);
            }
        }
        let hooks = format!("core.hooksPath={}", self.template.display());
        let output = command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.configuration)
            .args(["-c", &hooks, "-c", "core.longpaths=true"])
            .args(args)
            .output()
            .map_err(|error| {
                Unrun::error(error.into(), "running isolated copy operation".into())
            })?;
        Ok(output)
    }
}
