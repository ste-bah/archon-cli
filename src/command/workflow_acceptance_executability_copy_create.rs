//! Copy construction with explicit live-data exclusions.
use super::*;
impl HermeticCopy {
    /// `repository` at `commit` and `project`'s data, under `parent`.
    pub(super) fn at(
        parent: &Path,
        repository: &Path,
        project: &Path,
        task_root: Option<&Path>,
        commit: &str,
    ) -> Result<Self, Unrun> {
        Self::at_excluding(parent, repository, project, task_root, commit, &[])
    }
    pub(super) fn at_excluding(
        parent: &Path,
        repository: &Path,
        project: &Path,
        task_root: Option<&Path>,
        commit: &str,
        excludes: &[PathBuf],
    ) -> Result<Self, Unrun> {
        Self::create(
            parent, repository, project, task_root, commit, excludes, false, None,
        )
    }
    pub(super) fn create(
        parent: &Path,
        repository: &Path,
        project: &Path,
        task_root: Option<&Path>,
        commit: &str,
        excludes: &[PathBuf],
        isolated: bool,
        transport: Option<&Path>,
    ) -> Result<Self, Unrun> {
        let canonical = |path: &Path| {
            archon_shell::paths::canonicalize(path).map_err(|error| {
                let text = format!("{}: {error}", path.display());
                Unrun::error(error.into(), text)
            })
        };
        let (source, live_project) = (canonical(repository)?, canonical(project)?);
        std::fs::create_dir_all(parent).map_err(|error| {
            let text = format!("{}: {error}", parent.display());
            Unrun::error(error.into(), text)
        })?;
        let root = canonical(parent)?.join(format!("{COPY_PREFIX}{}", uuid::Uuid::new_v4()));
        for live in [&source, &live_project] {
            if root.starts_with(live) {
                return Err(Unrun::unknown(format!(
                    "the probe copy parent {} lies inside the live root {}",
                    parent.display(),
                    live.display()
                )));
            }
        }
        owner::write(&root)?;
        let mut copy = Self {
            repository: root.join("repo"),
            project: root.join("project"),
            root,
            removed: false,
            base_guard: None,
            isolated_git: None,
            prefix: PathBuf::new(),
            modified: Mutex::new(BTreeMap::new()),
        };
        let mut source_prefix = PathBuf::new();
        if isolated {
            let top = base_copy::output(&source, &["rev-parse", "--show-toplevel"])?;
            let live_top = canonical(Path::new(top.trim()))?;
            let transport = transport.unwrap_or(&live_top);
            source_prefix = source
                .strip_prefix(&live_top)
                .map_err(|error| Unrun::unknown(error.to_string()))?
                .to_path_buf();
            let object_format = base_copy::output(&source, &["rev-parse", "--show-object-format"])?;
            let isolated = base_git::IsolatedGit::new(&copy.root)?;
            let template_option = format!("--template={}", isolated.template().display());
            let format_option = format!("--object-format={}", object_format.trim());
            isolated.run(&[
                "init".as_ref(),
                "--quiet".as_ref(),
                format_option.as_ref(),
                template_option.as_ref(),
                copy.repository.as_os_str(),
            ])?;
            let fetched = isolated.output(&[
                "-C".as_ref(),
                copy.repository.as_os_str(),
                "-c".as_ref(),
                "core.longpaths=true".as_ref(),
                "fetch".as_ref(),
                "--no-tags".as_ref(),
                "--depth=1".as_ref(),
                transport.as_os_str(),
                commit.as_ref(),
            ])?;
            if !fetched.status.success() {
                return Err(match base_copy::baseline_available(&source, commit) {
                        Err(unavailable) if unavailable.2 == Some(crate::command::workflow_host_probe_fault::UnrunCause::BaselineUnavailable) => unavailable,
                        _ => Unrun::refused(
                            crate::command::workflow_host_probe_fault::UnrunCause::BaselineFetchFailed,
                            String::from_utf8_lossy(&fetched.stderr).trim().into(),
                        ),
                    });
            }
            isolated.run(&[
                "-C".as_ref(),
                copy.repository.as_os_str(),
                "checkout".as_ref(),
                "--quiet".as_ref(),
                "--detach".as_ref(),
                "FETCH_HEAD".as_ref(),
            ])?;
            copy.isolated_git = Some(isolated);
            copy.prefix = source_prefix.clone();
        } else {
            git(&[
                "clone".as_ref(),
                "--quiet".as_ref(),
                "--shared".as_ref(),
                "--no-checkout".as_ref(),
                "--".as_ref(),
                source.as_os_str(),
                copy.repository.as_os_str(),
            ])?;
            git(&[
                "-C".as_ref(),
                copy.repository.as_os_str(),
                "checkout".as_ref(),
                "--quiet".as_ref(),
                "--detach".as_ref(),
                commit.as_ref(),
                "--".as_ref(),
            ])?;
        }
        if isolated && !source_prefix.as_os_str().is_empty() {
            copy.repository = copy.repository.join(source_prefix);
            std::fs::create_dir_all(&copy.repository).map_err(|error| {
                Unrun::error(error.into(), "creating selected tree root".into())
            })?;
        }
        let tracked = live_project
            .strip_prefix(&source)
            .ok()
            .map(|relative| copy.repository.join(relative))
            .filter(|inside| inside.is_dir());
        match tracked {
            Some(inside) => copy.project = inside,
            None => {
                std::fs::create_dir_all(&copy.project)
                    .map_err(|e| Unrun::error(e.into(), "creating project copy".into()))?;
                let task_root = task_root.map(canonical).transpose()?;
                let task_root_relative = task_root
                    .as_deref()
                    .and_then(|root| root.strip_prefix(&live_project).ok());
                let before = data_digest_scoped(
                    &live_project,
                    &source,
                    task_root.as_deref(),
                    Path::new(""),
                    excludes,
                )?;
                let mut remaining = PROJECT_COPY_BYTES;
                copy_data_scoped(
                    &live_project,
                    Path::new(""),
                    &copy.project,
                    &source,
                    task_root_relative,
                    excludes,
                    &mut remaining,
                )?;
                if data_digest_scoped(
                    &live_project,
                    &source,
                    task_root.as_deref(),
                    Path::new(""),
                    excludes,
                )? != before
                {
                    return Err(Unrun::transient(
                        "the project data changed while it was copied, so the copy is no evidence of any one tree".into(),
                    ));
                }
            }
        }
        Ok(copy)
    }
}
