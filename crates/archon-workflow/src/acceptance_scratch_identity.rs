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
impl BuildIdentity {
    /// The parts of `other` that differ from this identity, named as
    /// [`super::faults::check_fault`] reads them.
    pub(super) fn changed_parts(&self, other: &Self) -> Vec<&'static str> {
        [
            (self.source != other.source, "source"),
            (self.target != other.target, "target"),
            (self.cwd_targets != other.cwd_targets, "target links"),
            (self.environment != other.environment, "environment"),
            (self.tools != other.tools, "tools"),
            (
                self.cargo_configuration != other.cargo_configuration,
                "build configuration",
            ),
        ]
        .into_iter()
        .filter_map(|(changed, part)| changed.then_some(part))
        .collect()
    }
    /// The parts of `other`, captured at a site laid out at `other_at`
    /// (root, build output), that differ from this identity captured at
    /// `at` on what does not depend on the site's path: the build output
    /// object and its links are the site's own and never compared; site
    /// paths in the environment are compared by what they name.
    pub(super) fn host_parts_changed(
        &self,
        at: (&Path, &Path),
        other: &Self,
        other_at: (&Path, &Path),
    ) -> Vec<&'static str> {
        let environment = |identity: &Self, (root, target): (&Path, &Path)| {
            (identity.environment.iter())
                .map(|(name, value)| (name.clone(), neutral(value, root, target)))
                .collect::<BTreeMap<_, _>>()
        };
        [
            (self.source != other.source, "source"),
            (
                environment(self, at) != environment(other, other_at),
                "environment",
            ),
            (self.tools != other.tools, "tools"),
            (
                self.cargo_configuration != other.cargo_configuration,
                "build configuration",
            ),
        ]
        .into_iter()
        .filter_map(|(changed, part)| changed.then_some(part))
        .collect()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckEvidence {
    pub acceptance_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sibling_revisions: Vec<crate::acceptance_sibling_paths::SiblingRevision>,
    pub before_identity: BuildIdentity,
    pub after_identity: Option<BuildIdentity>,
    pub changed_project_paths: Vec<String>,
    pub cargo_cache_before: String,
    pub cargo_cache_after: Option<String>,
    pub input_reset: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_sibling_record_serializes_like_the_main_identity_shape() {
        // EXPECTED before implementation: empty sibling evidence is omitted,
        // preserving the original record's serialized shape.
        let identity = BuildIdentity {
            source: BTreeMap::new(),
            target: "target".into(),
            cwd_targets: Vec::new(),
            environment: BTreeMap::new(),
            tools: BTreeMap::new(),
            cargo_configuration: BTreeMap::new(),
        };
        let record = CheckEvidence {
            acceptance_id: "AC-1".into(),
            sibling_revisions: Vec::new(),
            before_identity: identity,
            after_identity: None,
            changed_project_paths: Vec::new(),
            cargo_cache_before: "cache".into(),
            cargo_cache_after: None,
            input_reset: false,
        };
        let encoded = serde_json::to_string(&record).unwrap();
        assert!(!encoded.contains("sibling_revisions"), "{encoded}");
        let main_shape = r#"{"acceptance_id":"AC-1","before_identity":{"source":{},"target":"target","cwd_targets":[],"environment":{},"tools":{},"cargo_configuration":{}},"after_identity":null,"changed_project_paths":[],"cargo_cache_before":"cache","cargo_cache_after":null,"input_reset":false}"#;
        assert_eq!(encoded, main_shape);
    }
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
/// Why a site's identity could not be captured.
#[derive(Debug)]
pub(super) enum CaptureFailure {
    /// A part of the site is gone. After a check this is the check's own
    /// doing: the capture just before its run found the part, and only that
    /// run used the site between the two (classified by the typed error,
    /// never by its text).
    Removed { part: &'static str, path: PathBuf },
    /// A cwd's build output link no longer names the site's build output.
    TargetMapping,
    /// Anything else.
    Other(WorkflowError),
}
impl CaptureFailure {
    pub(super) fn of(roots: &ScratchRoots, part: &'static str, error: WorkflowError) -> Self {
        let path = match &error {
            WorkflowError::Io { path, source }
                if source.kind() == std::io::ErrorKind::NotFound
                    && (path.starts_with(roots.root()) || path.starts_with(roots.target())) =>
            {
                Some(path.clone())
            }
            // A tool reading the site (git) fails on a part it lost
            // without saying which: the site names it.
            _ => removed_part(roots),
        };
        match path {
            Some(path) => Self::Removed { part, path },
            None => Self::Other(error),
        }
    }
    pub(super) fn into_error(self) -> WorkflowError {
        match self {
            Self::Removed { part, path } => invalid(format!(
                "native scratch {part} missing at {}",
                path.display()
            )),
            Self::TargetMapping => invalid(super::faults::TARGET_MAPPING_CHANGED),
            Self::Other(error) => error,
        }
    }
}
/// The first part every site has (its root, repository copy and that
/// copy's git metadata, project copy, build output) that is gone.
pub(super) fn removed_part(roots: &ScratchRoots) -> Option<PathBuf> {
    [
        roots.root().to_path_buf(),
        roots.repository().to_path_buf(),
        roots.repository().join(".git"),
        roots.project().to_path_buf(),
        roots.target(),
    ]
    .into_iter()
    .find(|path| {
        path.symlink_metadata()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    })
}
pub(super) fn capture(
    roots: &ScratchRoots,
    policy: &ScratchPolicy,
) -> WorkflowResult<BuildIdentity> {
    capture_site(roots, policy).map_err(CaptureFailure::into_error)
}
/// [`capture`], saying which part of the site is gone when one is.
pub(super) fn capture_site(
    roots: &ScratchRoots,
    policy: &ScratchPolicy,
) -> Result<BuildIdentity, CaptureFailure> {
    let site = |part| move |error| CaptureFailure::of(roots, part, error);
    let mut targets = Vec::new();
    for cwd in [roots.project(), roots.repository()] {
        let path = cwd.join("target");
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| WorkflowError::io(&path, e))
            .map_err(site("target links"))?;
        if !meta.file_type().is_symlink()
            || path.canonicalize().map(archon_shell::paths::plain).ok() != Some(roots.target())
        {
            return Err(CaptureFailure::TargetMapping);
        }
        targets.push(object(&path).map_err(site("target links"))?);
    }
    let mut tools = BTreeMap::new();
    for name in ["cargo", "rustc", "python3", "python", "sh"] {
        if let Some(path) = std::env::split_paths(&policy.toolchain_path)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
        {
            let path = path
                .canonicalize()
                .map(archon_shell::paths::plain)
                .map_err(|e| CaptureFailure::Other(WorkflowError::io(&path, e)))?;
            let bytes = io::read(&path).map_err(CaptureFailure::Other)?;
            tools.insert(
                name.into(),
                format!("{}:{}", path.display(), content_digest(&bytes)),
            );
        }
    }
    let mut config = BTreeMap::new();
    for (label, base) in [
        ("cargo-home", roots.root().join("cargo-home")),
        (SITE_PROJECT_DIRECTORY, roots.project().join(".cargo")),
        (SITE_REPOSITORY_DIRECTORY, roots.repository().join(".cargo")),
    ] {
        for name in ["config", "config.toml", "credentials", "credentials.toml"] {
            let path = base.join(name);
            if path.symlink_metadata().is_ok() {
                let bytes = io::read(&path).map_err(site("build configuration"))?;
                config.insert(format!("{label}/{name}"), content_digest(&bytes));
            }
        }
    }
    Ok(BuildIdentity {
        source: roots.source_inventory().map_err(site("source"))?,
        target: object(&roots.target()).map_err(site("target"))?,
        cwd_targets: targets,
        // Never one machine's: its user and computer names stay out.
        environment: (roots.environment(policy).into_iter())
            .filter(|(name, _)| {
                !(crate::acceptance_check_environment::MACHINE_VARIABLES.iter())
                    .any(|machine| machine.eq_ignore_ascii_case(name))
            })
            .collect(),
        tools,
        cargo_configuration: config,
    })
}
/// `value` with the site's root and build output paths named, not spelled.
pub(super) fn neutral(value: &str, root: &Path, target: &Path) -> String {
    value
        .replace(&*target.to_string_lossy(), "<build output>")
        .replace(&*root.to_string_lossy(), "<site>")
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
