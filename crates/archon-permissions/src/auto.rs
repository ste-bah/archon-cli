use std::path::{Path, PathBuf};

use crate::classifier::{classify_command, CommandClass};
use crate::rules::{check_write_path, PathDecision};

// ---------------------------------------------------------------------------
// Decision type
// ---------------------------------------------------------------------------

/// The result of an auto-mode evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoDecision {
    /// The operation is safe and can proceed without prompting.
    Allow,
    /// The operation is risky and the user should be prompted.
    Prompt,
    /// The operation is dangerous; prompt with an explicit warning.
    PromptWithWarning(String),
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for auto-mode permission evaluation.
#[derive(Debug, Clone)]
pub struct AutoModeConfig {
    /// Additional commands the user considers safe.
    pub safe_commands: Vec<String>,
    /// Additional commands the user considers risky.
    pub risky_commands: Vec<String>,
    /// Additional commands the user considers dangerous.
    pub dangerous_commands: Vec<String>,
    /// Glob patterns for paths that are always allowed for writes.
    pub allow_paths: Vec<String>,
    /// Glob patterns for paths that are always denied for writes.
    pub deny_paths: Vec<String>,
    /// The project root directory. Writes inside this directory are allowed.
    pub project_dir: Option<PathBuf>,
}

impl Default for AutoModeConfig {
    fn default() -> Self {
        Self {
            safe_commands: Vec::new(),
            risky_commands: Vec::new(),
            dangerous_commands: Vec::new(),
            allow_paths: Vec::new(),
            deny_paths: Vec::new(),
            project_dir: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

/// Evaluates tool operations under auto-mode rules.
///
/// Safe operations are auto-approved, risky ones prompt the user, and
/// dangerous ones prompt with an explicit warning message.
pub struct AutoModeEvaluator {
    config: AutoModeConfig,
}

impl AutoModeEvaluator {
    pub fn new(config: AutoModeConfig) -> Self {
        Self { config }
    }

    /// Evaluate a shell command and return an [`AutoDecision`].
    pub fn evaluate_command(&self, command: &str) -> AutoDecision {
        let class = classify_command(
            command,
            &self.config.safe_commands,
            &self.config.risky_commands,
            &self.config.dangerous_commands,
        );

        match class {
            CommandClass::Safe => AutoDecision::Allow,
            CommandClass::Risky => AutoDecision::Prompt,
            CommandClass::Dangerous => AutoDecision::PromptWithWarning(format!(
                "Dangerous command detected: {command}"
            )),
        }
    }

    /// Evaluate a file read. Reads are always allowed in auto-mode.
    pub fn evaluate_file_read(&self, _path: &Path) -> AutoDecision {
        AutoDecision::Allow
    }

    /// Evaluate a file write using project-dir and path rules.
    ///
    /// - Writes to denied paths are promoted to a warning prompt.
    /// - Writes inside the project directory (or explicit allow list) are allowed.
    /// - Writes outside the project directory require a prompt.
    pub fn evaluate_file_write(&self, path: &Path) -> AutoDecision {
        let project_dir = self
            .config
            .project_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("."));

        let decision = check_write_path(
            path,
            &project_dir,
            &self.config.allow_paths,
            &self.config.deny_paths,
        );

        match decision {
            PathDecision::Allow => AutoDecision::Allow,
            PathDecision::NeedsPermission => AutoDecision::Prompt,
            PathDecision::Deny => AutoDecision::PromptWithWarning(format!(
                "Write to denied path: {}",
                path.display()
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_evaluator() -> AutoModeEvaluator {
        AutoModeEvaluator::new(AutoModeConfig {
            project_dir: Some(PathBuf::from("/home/user/project")),
            ..Default::default()
        })
    }

    #[test]
    fn safe_command_is_allowed() {
        let eval = default_evaluator();
        assert_eq!(eval.evaluate_command("ls -la"), AutoDecision::Allow);
    }

    #[test]
    fn risky_command_prompts() {
        let eval = default_evaluator();
        assert_eq!(eval.evaluate_command("cargo build"), AutoDecision::Prompt);
    }

    #[test]
    fn dangerous_command_warns() {
        let eval = default_evaluator();
        let decision = eval.evaluate_command("rm -rf /tmp/stuff");
        assert!(matches!(decision, AutoDecision::PromptWithWarning(_)));
    }

    #[test]
    fn unknown_command_is_risky() {
        let eval = default_evaluator();
        assert_eq!(eval.evaluate_command("some_unknown_tool"), AutoDecision::Prompt);
    }

    #[test]
    fn file_read_always_allowed() {
        let eval = default_evaluator();
        assert_eq!(
            eval.evaluate_file_read(Path::new("/etc/passwd")),
            AutoDecision::Allow
        );
    }

    #[test]
    fn file_write_inside_project_allowed() {
        let eval = default_evaluator();
        assert_eq!(
            eval.evaluate_file_write(Path::new("/home/user/project/src/main.rs")),
            AutoDecision::Allow
        );
    }

    #[test]
    fn file_write_outside_project_prompts() {
        let eval = default_evaluator();
        assert_eq!(
            eval.evaluate_file_write(Path::new("/tmp/other/file.txt")),
            AutoDecision::Prompt
        );
    }

    #[test]
    fn file_write_to_denied_path_warns() {
        let eval = AutoModeEvaluator::new(AutoModeConfig {
            project_dir: Some(PathBuf::from("/home/user/project")),
            deny_paths: vec!["*.env".to_string()],
            ..Default::default()
        });
        let decision = eval.evaluate_file_write(Path::new("production.env"));
        assert!(matches!(decision, AutoDecision::PromptWithWarning(_)));
    }

    #[test]
    fn file_write_to_allowed_path_outside_project() {
        let eval = AutoModeEvaluator::new(AutoModeConfig {
            project_dir: Some(PathBuf::from("/home/user/project")),
            allow_paths: vec!["/tmp/scratch/*".to_string()],
            ..Default::default()
        });
        assert_eq!(
            eval.evaluate_file_write(Path::new("/tmp/scratch/notes.txt")),
            AutoDecision::Allow
        );
    }

    #[test]
    fn user_safe_override_allows_command() {
        let eval = AutoModeEvaluator::new(AutoModeConfig {
            safe_commands: vec!["docker build".to_string()],
            project_dir: Some(PathBuf::from("/project")),
            ..Default::default()
        });
        assert_eq!(eval.evaluate_command("docker build -t myapp ."), AutoDecision::Allow);
    }

    #[test]
    fn user_dangerous_override_warns() {
        let eval = AutoModeEvaluator::new(AutoModeConfig {
            dangerous_commands: vec!["npm publish".to_string()],
            project_dir: Some(PathBuf::from("/project")),
            ..Default::default()
        });
        let decision = eval.evaluate_command("npm publish --access public");
        assert!(matches!(decision, AutoDecision::PromptWithWarning(_)));
    }

    #[test]
    fn pipe_chain_uses_worst_classification() {
        let eval = default_evaluator();
        // "ls" is safe, "rm -rf" is dangerous => dangerous wins
        let decision = eval.evaluate_command("ls | rm -rf /tmp");
        assert!(matches!(decision, AutoDecision::PromptWithWarning(_)));
    }

    #[test]
    fn empty_command_is_risky() {
        let eval = default_evaluator();
        assert_eq!(eval.evaluate_command(""), AutoDecision::Prompt);
    }

    #[test]
    fn default_config_has_empty_lists() {
        let cfg = AutoModeConfig::default();
        assert!(cfg.safe_commands.is_empty());
        assert!(cfg.risky_commands.is_empty());
        assert!(cfg.dangerous_commands.is_empty());
        assert!(cfg.allow_paths.is_empty());
        assert!(cfg.deny_paths.is_empty());
        assert!(cfg.project_dir.is_none());
    }
}
