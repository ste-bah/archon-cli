//! `archon config dump` — the effective configuration, in one place (#189 Phase 7).
//!
//! Config loading is a single TOML file, so there is no layering to resolve.
//! The problem is elsewhere: 157 distinct `ARCHON_*` variables are referenced
//! across the workspace and **not one** is read inside the config module. Every
//! one is read ad hoc at its point of use, so nothing in the codebase knows the
//! effective configuration, and answering "why is it behaving like this" means
//! grepping for the variable by hand.
//!
//! This prints the three things that answer it: which file was loaded, what it
//! resolved to, and which `ARCHON_*` variables are actually set in this
//! process. Output is redacted, because it is going to end up pasted into an
//! issue.

use std::collections::BTreeMap;

use archon_core::config::ArchonConfig;

include!(concat!(env!("OUT_DIR"), "/archon_env_vars.rs"));

/// Where the config came from, which is half the answer on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// Read from an existing file.
    Loaded,
    /// The file was absent, so defaults are in force and a template was written.
    CreatedFromTemplate,
}

impl ConfigOrigin {
    fn describe(&self) -> &'static str {
        match self {
            Self::Loaded => "loaded from disk",
            Self::CreatedFromTemplate => {
                "not present — defaults are in force and a template was written"
            }
        }
    }
}

/// One `ARCHON_*` variable set in this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvEntry {
    pub name: String,
    /// Redacted value.
    pub value: String,
    /// Whether anything in this build reads this name.
    pub recognised: bool,
}

/// Render the whole dump.
pub fn render(
    config_path: &std::path::Path,
    origin: &ConfigOrigin,
    config: &ArchonConfig,
    env: &[EnvEntry],
) -> String {
    let mut out = String::new();
    out.push_str("# Effective configuration\n\n");
    out.push_str(&format!("config file: {}\n", config_path.display()));
    out.push_str(&format!("status:      {}\n\n", origin.describe()));

    out.push_str("## Resolved config\n\n");
    match toml::to_string_pretty(config) {
        Ok(rendered) => {
            out.push_str(&archon_observability::redaction::redact_text(&rendered));
        }
        Err(error) => out.push_str(&format!("<could not serialise config: {error}>\n")),
    }

    out.push_str("\n## ARCHON_* environment\n\n");
    if env.is_empty() {
        out.push_str("(none set)\n");
        return out;
    }
    let width = env.iter().map(|entry| entry.name.len()).max().unwrap_or(0);
    for entry in env {
        let flag = if entry.recognised {
            ""
        } else {
            "  <- unrecognised"
        };
        out.push_str(&format!(
            "{:width$}  {}{flag}\n",
            entry.name,
            entry.value,
            width = width
        ));
    }
    if env.iter().any(|entry| !entry.recognised) {
        out.push_str(
            "\nUnrecognised names are set in the environment but read nowhere in this build — \
             usually a typo, and typos here fail silently.\n",
        );
    }
    out
}

/// Collect and redact the `ARCHON_*` variables set in this process.
pub fn collect_env<I, K, V>(vars: I) -> Vec<EnvEntry>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let known: std::collections::HashSet<&str> = KNOWN_ARCHON_ENV_VARS.iter().copied().collect();
    let sorted: BTreeMap<String, String> = vars
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .filter(|(key, _)| key.starts_with("ARCHON_"))
        .collect();
    sorted
        .into_iter()
        .map(|(name, value)| EnvEntry {
            recognised: known.contains(name.as_str()),
            value: archon_observability::redaction::redact_text(&value),
            name,
        })
        .collect()
}

/// Produce the dump for the current process.
pub fn dump() -> anyhow::Result<String> {
    let path = archon_core::config::default_config_path();
    let origin = if path.exists() {
        ConfigOrigin::Loaded
    } else {
        ConfigOrigin::CreatedFromTemplate
    };
    let config = archon_core::config::load_config()
        .map_err(|error| anyhow::anyhow!("failed to load config: {error}"))?;
    let env = collect_env(std::env::vars());
    Ok(render(&path, &origin, &config, &env))
}

#[cfg(test)]
#[path = "config_dump_tests.rs"]
mod tests;
