//! Provider configuration for proof scratch projects.

use std::path::Path;

/// Copies the deployed project's provider configuration into a scratch project.
///
/// Fixed decomposition resolves its endpoint with `ConfiguredOnly`, which by
/// design ignores ambient `ANTHROPIC_BASE_URL` so a hostile parent environment
/// cannot redirect author prompts. The synthetic template carries only
/// `gate_mode`, so without this every author call in the proof goes to the
/// default endpoint with a placeholder key and fails operationally -- three
/// times, and the run ends `Failed` having authored nothing.
///
/// The values are inherited from the deployed project rather than written into
/// the committed fixture, so the proof runs against whatever provider the
/// operator actually runs, and no endpoint is baked into the repository.
pub fn inherit_provider_config(deployed: &Path, scratch: &Path) -> Result<(), String> {
    let source = deployed.join(".archon/config.toml");
    let text = std::fs::read_to_string(&source)
        .map_err(|error| format!("reading deployed config {}: {error}", source.display()))?;
    let value: toml::Value = text
        .parse()
        .map_err(|error| format!("parsing deployed config {}: {error}", source.display()))?;
    // Both sections, not just the endpoint. Agent code names models by tier
    // alias -- the fixed decomposition's author tier is hardcoded to "sonnet" --
    // and the alias is resolved against `[models.anthropic]`. Copying only
    // `[api]` gives the scratch project the right endpoint with the stock alias,
    // so every author call asks a local proxy for a model it does not serve and
    // fails with HTTP 400 before any authoring happens.
    let mut inherited = toml::value::Table::new();
    for section in ["api", "models"] {
        let Some(found) = value.get(section) else {
            if section == "api" {
                return Err(format!(
                    "deployed config {} has no [api] section",
                    source.display()
                ));
            }
            continue;
        };
        inherited.insert(section.to_string(), found.clone());
    }
    let rendered = toml::to_string(&inherited)
        .map_err(|error| format!("serialising inherited provider config: {error}"))?;

    let target = scratch.join(".archon/config.toml");
    let mut existing = std::fs::read_to_string(&target)
        .map_err(|error| format!("reading scratch config {}: {error}", target.display()))?;
    if !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str("\n# Inherited from the deployed project for this proof run.\n");
    existing.push_str(&rendered);
    std::fs::write(&target, existing)
        .map_err(|error| format!("writing scratch config {}: {error}", target.display()))
}


/// The proof's scratch project, kept on disk when asked.
///
/// A `TempDir` is deleted when the test unwinds, taking the authored workflow,
/// the call records and the decomposition log with it -- so a failure late in a
/// ninety-minute run leaves nothing to diagnose, and the next attempt is another
/// ninety minutes. Setting `ARCHON_R2A_KEEP_WORKSPACE` to a directory puts the
/// project there instead and leaves it in place.
pub enum ProofWorkspace {
    Temporary(tempfile::TempDir),
    Kept(std::path::PathBuf),
}

impl ProofWorkspace {
    pub fn create() -> Self {
        match std::env::var_os("ARCHON_R2A_KEEP_WORKSPACE") {
            Some(root) => {
                let root = std::path::PathBuf::from(root).join(format!(
                    "synthetic-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("clock")
                        .as_secs()
                ));
                std::fs::create_dir_all(&root).expect("proof workspace");
                eprintln!("proof workspace kept at {}", root.display());
                Self::Kept(root)
            }
            None => Self::Temporary(tempfile::tempdir().expect("synthetic scratch project")),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Temporary(dir) => dir.path(),
            Self::Kept(path) => path.as_path(),
        }
    }
}
