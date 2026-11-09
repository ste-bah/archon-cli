//! Configuration isolation shared by committed-tree copy consumers.
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error("{context}: {error}")]
    Io {
        error: std::io::Error,
        context: String,
    },
    #[error("{0}")]
    Operation(String),
}
pub struct IsolatedGit {
    configuration: PathBuf,
    template: PathBuf,
}
impl IsolatedGit {
    pub fn new(root: &Path) -> Result<Self, CopyError> {
        let io = |error, context: &str| CopyError::Io {
            error,
            context: context.into(),
        };
        std::fs::create_dir_all(root).map_err(|e| io(e, "creating isolated copy storage"))?;
        let configuration = root.join(uuid::Uuid::new_v4().to_string());
        let template = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&configuration, []).map_err(|e| io(e, "creating isolated configuration"))?;
        std::fs::create_dir(&template).map_err(|e| io(e, "creating isolated template"))?;
        Ok(Self {
            configuration,
            template,
        })
    }
    pub fn template(&self) -> &Path {
        &self.template
    }
    pub fn run(&self, args: &[&std::ffi::OsStr]) -> Result<(), CopyError> {
        self.output(args).map(|_| ())
    }
    pub fn output(&self, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, CopyError> {
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
            .map_err(|error| CopyError::Io {
                error,
                context: "running isolated copy operation".into(),
            })?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        let cause = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(CopyError::Operation(if cause.is_empty() {
            format!("isolated operation exited with {}", output.status)
        } else {
            cause
        }))
    }
}
