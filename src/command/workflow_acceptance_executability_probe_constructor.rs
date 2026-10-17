use super::*;

impl HostProbe {
    fn new_with_limits(
        project: PathBuf,
        repository: PathBuf,
        site: Site,
        resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
    ) -> Self {
        let check_cap_secs = probe_check_cap_secs(&site);
        Self {
            project,
            task_root: None,
            identity: std::sync::OnceLock::new(),
            data_states: Mutex::new(BTreeMap::new()),
            repository,
            site,
            host: archon_workflow::acceptance_check_environment::host_environment(),
            resource_limits,
            diagnostics: Mutex::new(Vec::new()),
            regression_guards: Mutex::new(BTreeMap::new()),
            unproven: Mutex::new(BTreeMap::new()),
            baseline: None,
            baseline_failures: Mutex::new(BTreeMap::new()),
            reuse_audit: Mutex::new(BTreeMap::new()),
            failed_tree: Mutex::new(None),
            copy_parent: std::env::temp_dir(),
            memo: false,
            resume: crate::command::workflow_freeze_budget::FreezeResume::none(),
            check_cap_secs,
            deferred: Mutex::new(BTreeSet::new()),
            #[cfg(test)]
            process_memo: true,
            #[cfg(test)]
            injected_failures: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            copies_made: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// At an acceptance round's own site: its scratch policy, else the live
    /// target repository the round runs its checks in.
    pub(crate) fn at_with_limits(
        project: PathBuf,
        repository: PathBuf,
        binding: Option<NativeBinding>,
        resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
    ) -> Self {
        let site = binding.map_or(Site::Direct, |binding| Site::Scratch(Box::new(binding)));
        Self::new_with_limits(project, repository, site, resource_limits)
    }

    /// Also prove every probed check fails on `baseline`, the tree before
    /// any implementation (see the module docs).
    pub(crate) fn with_baseline(mut self, baseline: Baseline) -> Self {
        self.baseline = Some(baseline);
        self
    }

    /// Also hold every probed check to its original's verdict on the tree
    /// that original ran on (A5, `FailedTree`).
    pub(crate) fn with_failed_tree(self, tree: FailedTree) -> Self {
        *self.failed_tree.lock().expect("failed tree lock") = Some(tree);
        self
    }

    /// At freeze time, before any run: the hermetic scratch site when one is
    /// configured (building warm from a per-repository cache), otherwise the
    /// probe's own hermetic copy -- never the live repository. A configured
    /// policy that cannot be captured, or a repository record that cannot be
    /// believed, runs nothing (`sites::freeze_site`). The probe carries the
    /// task set's pre-implementation baseline (`Baseline::for_task_set`).
    pub(crate) fn for_task_set_with_limits(
        project: &std::path::Path,
        tasks_root: &std::path::Path,
        resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
    ) -> Self {
        let (site, repository) = sites::freeze_site(project, tasks_root);
        let unavailable = matches!(site, Site::Unavailable(_));
        let mut probe =
            Self::new_with_limits(project.to_path_buf(), repository, site, resource_limits);
        probe.task_root = Some(tasks_root.to_path_buf());
        probe.memo = true;
        if unavailable {
            return probe;
        }
        probe.baseline = Baseline::for_task_set(&probe.repository, tasks_root);
        if let Site::Scratch(binding) = &probe.site
            && let Some(cache) = &binding.policy.build_cache
        {
            probe.note(format!(
                "the freeze probe builds warm from the scratch build cache {}",
                cache.display()
            ));
        }
        if probe.baseline.is_none() {
            probe.note(format!(
                "pre-implementation probe not run: {} is not a git checkout with a commit, so the checks are not proven able to fail",
                probe.repository.display()
            ));
        }
        probe
    }

    /// Reverify uses the same staged cache and host no-progress window as
    /// freeze-acceptance, while still probing the completed tree.
    pub(crate) fn for_reverify_with_limits(
        project: &std::path::Path,
        tasks_root: &std::path::Path,
        resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
        resume: &crate::command::workflow_freeze_budget::FreezeResume,
    ) -> Self {
        Self::for_task_set_with_limits(project, tasks_root, resource_limits).with_resume(resume)
    }

    #[cfg(test)]
    pub(super) fn new(project: PathBuf, repository: PathBuf, site: Site) -> Self {
        Self::new_with_limits(
            project,
            repository,
            site,
            archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn at(
        project: PathBuf,
        repository: PathBuf,
        binding: Option<NativeBinding>,
    ) -> Self {
        Self::at_with_limits(
            project,
            repository,
            binding,
            archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn for_task_set(project: &std::path::Path, tasks_root: &std::path::Path) -> Self {
        Self::for_task_set_with_limits(
            project,
            tasks_root,
            archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        )
    }

    pub(super) fn note(&self, text: String) {
        self.diagnostics
            .lock()
            .expect("diagnostics lock")
            .push(text);
    }

    pub(super) fn note_regression_guard(&self, id: &str, basis: String) {
        self.regression_guards
            .lock()
            .expect("regression guards lock")
            .insert(id.to_string(), basis);
    }

    /// Record that the host could not prove `id`.
    pub(super) fn unproven(&self, id: &str, why: String) {
        self.unproven
            .lock()
            .expect("unproven lock")
            .insert(id.to_string(), why);
    }
}
