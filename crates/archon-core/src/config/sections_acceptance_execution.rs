//! Opt-in native acceptance execution policy. No ambient environment inheritance.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceExecutionConfig {
    pub repository: PathBuf,
    pub scratch_parent: PathBuf,
    pub project_inputs: Vec<PathBuf>,
    #[serde(default)]
    pub project_input_excludes: Vec<PathBuf>,
    pub project_repository_view: AcceptanceProjectView,
    pub toolchain_path: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub environment_allowlist: Vec<String>,
    pub cargo_seed: Option<PathBuf>,
    pub timeout_secs: u64,
    pub output_bytes: usize,
    pub scratch_bytes: u64,
    /// Issue-226: absolute directories outside both the project and the
    /// repository under which a declared data root may be granted and
    /// landed. Empty: no external root is ever written.
    #[serde(default)]
    pub external_data_roots: Vec<PathBuf>,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceProjectView {
    #[default]
    Separate,
    Combined,
}

#[cfg(test)]
mod tests {
    use super::AcceptanceExecutionConfig;

    /// The documented contract (config.toml, `[workflow.acceptance_execution]`):
    /// the capacities are examples, not portable defaults, so a section that
    /// omits `timeout_secs` is refused, naming the field, never defaulted.
    #[test]
    fn a_section_without_timeout_secs_is_refused_by_name() {
        let section = r#"
repository = "/r"
scratch_parent = "/s"
project_inputs = []
project_repository_view = "separate"
toolchain_path = "/usr/bin:/bin"
output_bytes = 1048576
scratch_bytes = 10737418240
"#;
        let error = toml::from_str::<AcceptanceExecutionConfig>(section)
            .expect_err("timeout_secs is required")
            .to_string();
        assert!(error.contains("timeout_secs"), "{error}");
        let with = format!("{section}timeout_secs = 3600\n");
        let config: AcceptanceExecutionConfig = toml::from_str(&with).expect("complete section");
        assert_eq!(config.timeout_secs, 3600);
    }
}
