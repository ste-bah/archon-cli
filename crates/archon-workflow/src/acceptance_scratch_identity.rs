//! Build identity and per-check mutation evidence. Target contents may grow;
//! replacing its directory or cwd link is never warm-cache reuse.
use super::*;
use crate::task_set_contract::content_digest;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIdentity {
    source: BTreeMap<String, String>,
    target: String,
    cwd_targets: Vec<String>,
    environment: BTreeMap<String, String>,
    tools: BTreeMap<String, String>,
    cargo_configuration: BTreeMap<String, String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckEvidence {
    pub acceptance_id: String,
    pub before_identity: BuildIdentity,
    pub after_identity: Option<BuildIdentity>,
    pub changed_project_paths: Vec<String>,
    pub cargo_cache_before: String,
    pub cargo_cache_after: Option<String>,
    pub input_reset: bool,
}
fn object(path: &Path) -> WorkflowResult<String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| WorkflowError::io(path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("{}:{}:{}", m.dev(), m.ino(), m.mode()))
    }
    #[cfg(not(unix))]
    {
        Ok(format!("{:?}:{}", m.file_type(), m.len()))
    }
}
pub(super) fn capture(
    roots: &ScratchRoots,
    policy: &ScratchPolicy,
) -> WorkflowResult<BuildIdentity> {
    let mut targets = Vec::new();
    for cwd in [roots.project(), roots.repository()] {
        let path = cwd.join("target");
        let meta = std::fs::symlink_metadata(&path).map_err(|e| WorkflowError::io(&path, e))?;
        if !meta.file_type().is_symlink() || path.canonicalize().ok() != Some(roots.target()) {
            return Err(invalid("native build identity changed: target mapping"));
        }
        targets.push(object(&path)?);
    }
    let mut tools = BTreeMap::new();
    for name in ["cargo", "rustc", "python3", "python", "sh"] {
        if let Some(path) = policy
            .toolchain_path
            .split(':')
            .map(|p| Path::new(p).join(name))
            .find(|p| p.is_file())
        {
            let path = path
                .canonicalize()
                .map_err(|e| WorkflowError::io(&path, e))?;
            tools.insert(
                name.into(),
                format!("{}:{}", path.display(), content_digest(&io::read(&path)?)),
            );
        }
    }
    let mut config = BTreeMap::new();
    for name in ["config", "config.toml", "credentials", "credentials.toml"] {
        let path = roots.root().join("cargo-home").join(name);
        if path.exists() {
            config.insert(name.into(), content_digest(&io::read(&path)?));
        }
    }
    Ok(BuildIdentity {
        source: roots.source_inventory()?,
        target: object(&roots.target())?,
        cwd_targets: targets,
        environment: roots.environment(policy),
        tools,
        cargo_configuration: config,
    })
}
pub(super) fn cache_digest(roots: &ScratchRoots) -> WorkflowResult<String> {
    Ok(content_digest(&serde_json::to_vec(&inventory(
        &roots.root().join("cargo-home"),
    )?)?))
}
pub(super) fn changed(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|k| before.get(*k) != after.get(*k))
        .cloned()
        .collect()
}
