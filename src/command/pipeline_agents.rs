use std::path::Path;

use anyhow::{Result, bail};
use archon_core::agents::AgentRegistry;
use archon_pipeline::runner::PipelineType;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SyncSummary {
    pub(crate) created: usize,
    pub(crate) updated: usize,
    pub(crate) differing: Vec<std::path::PathBuf>,
    pub(crate) update_guidance: Option<&'static str>,
}

const UPDATE_GUIDANCE: &str = "Review these files, then run `archon agents sync --update` to replace all differing bundled files.";

/// Validate the full built-in agent set before any pipeline work begins.
pub(crate) fn preflight(cwd: &Path, kind: PipelineType) -> Result<()> {
    let required = archon_pipeline::required_builtin_agent_keys(kind.clone());
    if required.is_empty() {
        return Ok(());
    }
    let registry = AgentRegistry::load(cwd);
    let missing: Vec<_> = required
        .into_iter()
        .filter(|key| registry.resolve(key).is_none())
        .collect();
    if !missing.is_empty() {
        bail!(
            "Missing required {kind:?} pipeline agents: {}. Install them with `archon agents sync`.",
            missing.join(", ")
        );
    }
    Ok(())
}

/// Install missing bundled files and optionally replace differing files.
pub(crate) fn sync(cwd: &Path, update: bool) -> Result<SyncSummary> {
    use std::fs;
    use std::path::PathBuf;

    let mut summary = SyncSummary::default();
    let mut differing = Vec::<PathBuf>::new();
    for (relative, contents) in super::pipeline_agent_assets::all() {
        let path = cwd.join(relative);
        match fs::read_to_string(&path) {
            Ok(existing) if existing == contents => {}
            Ok(_) if update => {
                fs::write(&path, contents)?;
                println!("Updated {}", path.display());
                summary.updated += 1;
            }
            Ok(_) => differing.push(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, contents)?;
                println!("Installed {}", path.display());
                summary.created += 1;
            }
            Err(error) => return Err(error.into()),
        }
    }
    if !differing.is_empty() {
        for path in &differing {
            println!("Different (preserved): {}", path.display());
        }
        println!("{UPDATE_GUIDANCE}");
        summary.update_guidance = Some(UPDATE_GUIDANCE);
    }
    if summary.created == 0 && summary.updated == 0 && differing.is_empty() {
        println!("Pipeline agent files are already in sync.");
    }
    summary.differing = differing;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("archon-pipeline-agents-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn bundled_assets_resolve_every_required_key_through_registry() {
        let path = project();
        let bundled_paths: Vec<_> = super::super::pipeline_agent_assets::all()
            .map(|(path, _)| path)
            .collect();
        for kind in [
            PipelineType::Research,
            PipelineType::Coding,
            PipelineType::GameTheory,
        ] {
            for key in archon_pipeline::required_builtin_agent_keys(kind.clone()) {
                assert!(
                    bundled_paths
                        .iter()
                        .any(|path| path.ends_with(&format!("/{key}.md"))),
                    "embedded files must cover {key} from {kind:?}"
                );
            }
        }
        let first_sync = sync(&path, false).unwrap();
        assert_eq!(
            first_sync.created,
            super::super::pipeline_agent_assets::all().count()
        );
        let installed: Vec<_> = super::super::pipeline_agent_assets::all()
            .map(|(relative, _)| {
                let file = path.join(relative);
                let bytes = fs::read(&file).unwrap();
                let modified = fs::metadata(&file).unwrap().modified().unwrap();
                (file, bytes, modified)
            })
            .collect();
        let second_sync = sync(&path, false).unwrap();
        assert_eq!(second_sync.created, 0);
        assert_eq!(second_sync.updated, 0);
        assert!(second_sync.differing.is_empty());
        for (file, bytes, modified) in installed {
            assert_eq!(
                fs::read(&file).unwrap(),
                bytes,
                "bytes changed: {}",
                file.display()
            );
            assert_eq!(
                fs::metadata(&file).unwrap().modified().unwrap(),
                modified,
                "mtime changed: {}",
                file.display()
            );
        }
        let registry = AgentRegistry::load_with_user_home(&path, None);
        for kind in [
            PipelineType::Research,
            PipelineType::Coding,
            PipelineType::GameTheory,
        ] {
            for key in archon_pipeline::required_builtin_agent_keys(kind.clone()) {
                assert!(
                    registry.resolve(key).is_some(),
                    "{key} must resolve for {kind:?}"
                );
            }
        }
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn preflight_reports_every_missing_key_and_sync_preserves_then_updates_edits() {
        let path = project();
        let before = AgentRegistry::load_with_user_home(&path, None);
        let missing: Vec<_> = archon_pipeline::required_builtin_agent_keys(PipelineType::Coding)
            .into_iter()
            .filter(|key| before.resolve(key).is_none())
            .collect();
        let error = preflight(&path, PipelineType::Coding)
            .unwrap_err()
            .to_string();
        assert!(!missing.is_empty());
        for key in &missing {
            assert!(error.contains(key), "missing key {key} must be reported");
        }
        assert!(error.contains("archon agents sync"));

        let (relative, bundled) = super::super::pipeline_agent_assets::all().next().unwrap();
        let edited = path.join(relative);
        fs::create_dir_all(edited.parent().unwrap()).unwrap();
        fs::write(&edited, "user edit").unwrap();
        let preserved = sync(&path, false).unwrap();
        assert_eq!(fs::read_to_string(&edited).unwrap(), "user edit");
        assert!(
            preserved.differing.contains(&edited),
            "edited file must be reported"
        );
        assert!(
            preserved
                .update_guidance
                .unwrap()
                .contains("archon agents sync --update")
        );
        sync(&path, true).unwrap();
        assert_eq!(fs::read_to_string(&edited).unwrap(), bundled);
        sync(&path, false).unwrap();
        assert!(preflight(&path, PipelineType::Coding).is_ok());
        fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn audited_resume_preflights_before_pipeline_setup() {
        let path = project();
        let registry = AgentRegistry::load(&path);
        let missing: Vec<_> = archon_pipeline::required_builtin_agent_keys(PipelineType::Coding)
            .into_iter()
            .filter(|key| registry.resolve(key).is_none())
            .collect();
        assert!(!missing.is_empty());
        archon_pipeline::audit::store::PipelineBundleStore::new(&path)
            .create("resume-preflight", PipelineType::Coding, "resume task")
            .unwrap();

        let error = crate::command::pipeline_bundle::handle_resume(
            &path,
            "resume-preflight",
            &archon_core::config::ArchonConfig::default(),
            &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
            false,
        )
        .await
        .unwrap_err()
        .to_string();
        for key in missing {
            assert!(
                error.contains(&key),
                "missing key {key} must be reported: {error}"
            );
        }
        assert!(error.contains("archon agents sync"));
        fs::remove_dir_all(path).unwrap();
    }
}
