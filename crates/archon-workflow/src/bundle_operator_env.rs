//! Host-owned operator credential grants for a run.
use super::*;

impl WorkflowBundle {
    /// Capture only key names from the operator declaration, never values.
    pub fn create_for_run_with_operator_grants(
        store: &WorkflowStore,
        run: &WorkflowRun,
        harness_source: &str,
        origin: WorkflowBundleOrigin,
        operator_env_grants: &[String],
    ) -> WorkflowResult<Self> {
        persist_operator_grants(store, &run.id, operator_env_grants)?;
        Self::write_for_run_with_operator_grants(
            store,
            run,
            harness_source,
            origin,
            operator_env_grants,
        )
    }

    pub(super) fn write_for_run_with_operator_grants(
        store: &WorkflowStore,
        run: &WorkflowRun,
        harness_source: &str,
        origin: WorkflowBundleOrigin,
        operator_env_grants: &[String],
    ) -> WorkflowResult<Self> {
        #[derive(Serialize)]
        struct CompiledRecord<'a> {
            #[serde(flatten)]
            spec: &'a WorkflowSpec,
            operator_env_grants: &'a [String],
        }
        let compiled = serde_yaml_ng::to_string(&CompiledRecord {
            spec: &run.spec,
            operator_env_grants,
        })?;
        let manifest =
            manifest_for_run(run, harness_source.as_bytes(), compiled.as_bytes(), origin);
        store.write_run_file(
            &run.id,
            WorkflowRecordKind::of_source(harness_source).file_name(),
            harness_source.as_bytes(),
        )?;
        store.write_run_file(&run.id, COMPILED_SPEC_FILE, compiled.as_bytes())?;
        store.write_run_file(
            &run.id,
            MANIFEST_FILE,
            toml::to_string_pretty(&manifest)?.as_bytes(),
        )?;
        Ok(Self {
            run_id: run.id.clone(),
            run_dir: store.run_dir(&run.id),
            manifest,
        })
    }

    /// Project-local bundle and approval files confer no grant authority.
    /// Missing host records (including older runs) fail closed.
    pub fn verified_operator_env_grants(
        store: &WorkflowStore,
        run_id: &str,
    ) -> WorkflowResult<Vec<String>> {
        if !archon_shell::read_boundary::operator_grants_supported() {
            tracing::warn!(
                run_id,
                operator_env_grants = "disabled",
                "operator grants disabled for this run: kernel write containment unavailable"
            );
            return Ok(Vec::new());
        }
        let path = operator_grant_path(run_id)?;
        match std::fs::read(&path) {
            Ok(bytes) => {
                let record: OperatorGrantRecord = match serde_json::from_slice(&bytes) {
                    Ok(record) => record,
                    Err(_) => {
                        tracing::warn!(
                            run_id,
                            "host operator grant record has no valid identity binding; granting no keys"
                        );
                        return Ok(Vec::new());
                    }
                };
                let Ok((project_root, run_dir)) = canonical_grant_binding(store, run_id) else {
                    tracing::warn!(
                        run_id,
                        "host operator grant paths cannot be resolved; granting no keys"
                    );
                    return Ok(Vec::new());
                };
                if record.run_id != run_id
                    || record.project_root != project_root
                    || record.run_dir != run_dir
                {
                    tracing::warn!(
                        run_id,
                        "host operator grant identity/project root/run directory mismatch; granting no keys"
                    );
                    return Ok(Vec::new());
                }
                Ok(record.grants)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(run_id, path = %path.display(), "host operator environment grant record missing; granting no keys");
                Ok(Vec::new())
            }
            Err(error) => {
                tracing::warn!(
                    run_id,
                    "host operator environment grant record unreadable; granting no keys"
                );
                Err(WorkflowError::io(&path, error))
            }
        }
    }
}

fn operator_grant_path(run_id: &str) -> WorkflowResult<PathBuf> {
    if run_id.is_empty()
        || !run_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err(WorkflowError::ArtifactInvalid(
            "invalid run id for host grant record".into(),
        ));
    }
    let root = archon_shell::credential_paths::operator_grant_root();
    if !root.is_absolute() {
        return Err(WorkflowError::ArtifactInvalid(
            "host home is unavailable; granting no environment keys".into(),
        ));
    }
    Ok(root.join(format!("{run_id}.json")))
}

#[derive(Serialize, Deserialize)]
struct OperatorGrantRecord {
    run_id: String,
    project_root: PathBuf,
    run_dir: PathBuf,
    grants: Vec<String>,
}

fn canonical_grant_binding(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<(PathBuf, PathBuf)> {
    let project = crate::approval::project_root_from_workflow_root(store.root());
    let canonical = |path: &Path| {
        path.canonicalize()
            .map(archon_shell::paths::plain)
            .map_err(|error| WorkflowError::io(path, error))
    };
    Ok((canonical(&project)?, canonical(&store.run_dir(run_id))?))
}

fn persist_operator_grants(
    store: &WorkflowStore,
    run_id: &str,
    grants: &[String],
) -> WorkflowResult<()> {
    use std::io::Write;
    let supported = archon_shell::read_boundary::operator_grants_supported();
    tracing::info!(
        run_id,
        operator_env_grants = if supported {
            "enabled"
        } else {
            "disabled: kernel write containment unavailable"
        },
        "workflow launch authority"
    );
    // A missing host record already grants nothing. Ordinary runs and hosts
    // that cannot issue grants must not depend on writable credential storage.
    if grants.is_empty() || !supported {
        return Ok(());
    }
    let (project_root, run_dir) = canonical_grant_binding(store, run_id)?;
    let record = OperatorGrantRecord {
        run_id: run_id.to_owned(),
        project_root,
        run_dir,
        grants: grants.to_vec(),
    };
    let path = operator_grant_path(run_id)?;
    let root = path.parent().expect("host grant root");
    std::fs::create_dir_all(root).map_err(|error| WorkflowError::io(root, error))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // One launch, one record. Existing authority is never replaced by a bundle rewrite.
    let mut file = options
        .open(&path)
        .map_err(|error| WorkflowError::io(&path, error))?;
    file.write_all(&serde_json::to_vec(&record)?)
        .map_err(|error| WorkflowError::io(&path, error))?;
    file.sync_all()
        .map_err(|error| WorkflowError::io(&path, error))
}

#[cfg(test)]
#[path = "bundle_operator_env_tests.rs"]
mod tests;
