//! Tearing a scratch site down: once when its observation ends, or between
//! two checks of one observation, where the build cache lease is kept so
//! no other holder can take the cache while the observation still runs.
use super::*;

impl ScratchRoots {
    pub fn cleanup(&mut self) -> WorkflowResult<()> {
        self.teardown()?;
        self.lease = None;
        Ok(())
    }

    /// Tear this site down and hand back its build cache lease, still
    /// held, for the site that replaces it ([`Self::prepare_leased`]): an
    /// observation holds the cache's lock for its whole life.
    pub(super) fn tear_down_keeping_lease(&mut self) -> WorkflowResult<Option<cache::Lease>> {
        self.teardown()?;
        Ok(self.lease.take())
    }

    fn teardown(&mut self) -> WorkflowResult<()> {
        let control = control::Control::new(
            self.cleanup_timeout_secs,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        control.run(|| self.teardown_inner())
    }

    fn teardown_inner(&mut self) -> WorkflowResult<()> {
        if self.cleaned {
            return Ok(());
        }
        // Before the slot goes: a check that touched a tracked file forgets
        // the cache's builds. Should that fail, the slot stays, and the next
        // holder, finding it, forgets them instead.
        if let Some(lease) = &mut self.lease
            && self.registered
        {
            lease.audit(&[
                (SITE_REPOSITORY_DIRECTORY, &self.repository),
                (SITE_PROJECT_DIRECTORY, &self.project),
            ])?;
        }
        if self.registered {
            self.relink()?;
            if let Err(error) = git(
                &self.live_repository,
                &["worktree", "remove", "--force"],
                &[&self.repository],
            ) {
                let text = error.to_string();
                let path = self.repository.to_string_lossy();
                let check_removal = text.contains("failed to delete")
                    && text.contains(path.as_ref())
                    && (text.contains("Operation not permitted")
                        || text.contains("Permission denied"));
                if check_removal {
                    if let Ok(listed) = git(
                        &self.live_repository,
                        &["worktree", "list", "--porcelain"],
                        &[],
                    ) {
                        let expected = format!("worktree {}", self.repository.display());
                        self.registered = listed.lines().any(|line| line == expected);
                    }
                    if !self.registered {
                        return Err(WorkflowError::ScratchSiteRemoval(text));
                    }
                }
                return Err(error);
            }
            let listed = git(
                &self.live_repository,
                &["worktree", "list", "--porcelain"],
                &[],
            )?;
            let expected = format!("worktree {}", self.repository.display());
            if listed.lines().any(|line| line == expected) {
                return Err(invalid(
                    "owned scratch worktree remains registered after removal",
                ));
            }
            self.registered = false;
        }
        // Release the site's own root handle before removing the root. On
        // Windows a directory this process holds open without delete
        // sharing cannot be removed (ERROR_SHARING_VIOLATION, os error 32);
        // a Unix descriptor never blocks removal. No check runs on a site
        // being torn down, and a retried teardown needs no handle.
        self.mutation_root = None;
        if let Err(error) = io::remove_owned_tree(&self.root) {
            if let WorkflowError::Io { path, source } = &error
                && path.starts_with(&self.root)
                && source.kind() == std::io::ErrorKind::PermissionDenied
            {
                return Err(WorkflowError::ScratchSiteRemoval(error.to_string()));
            }
            return Err(error);
        }
        self.cleaned = true;
        Ok(())
    }

    /// A check that removed or replaced the repository copy's `.git` link
    /// leaves a worktree the live repository refuses to remove: put the
    /// link it wrote back first. A copy removed whole needs none.
    fn relink(&self) -> WorkflowResult<()> {
        let copy = self.repository.symlink_metadata();
        let (Some(bytes), true) = (&self.worktree_link, copy.is_ok_and(|m| m.is_dir())) else {
            return Ok(());
        };
        let link = self.repository.join(".git");
        if std::fs::read(&link).is_ok_and(|held| &held == bytes) {
            return Ok(());
        }
        if link.symlink_metadata().is_ok() {
            io::remove_owned_tree(&link)?;
        }
        std::fs::write(&link, bytes).map_err(|e| WorkflowError::io(&link, e))
    }

    /// Whether the `.git` file at `link` names this site's own worktree:
    /// the administrative directory it names records this site's
    /// repository copy as its worktree. That directory's name is the live
    /// repository's choice, so it is never compared.
    pub(super) fn links_own_worktree(&self, link: &Path) -> bool {
        let named = |file: &Path, prefix: &str| {
            let text = std::fs::read_to_string(file).ok()?;
            let path = Path::new(text.trim_end().strip_prefix(prefix)?);
            Some(file.parent()?.join(path))
        };
        let canonical = |path: &Path| std::fs::canonicalize(path).map(archon_shell::paths::plain);
        let worktree = named(link, "gitdir: ").and_then(|admin| named(&admin.join("gitdir"), ""));
        match (worktree, canonical(&self.repository.join(".git"))) {
            (Some(worktree), Ok(own)) => canonical(&worktree).is_ok_and(|named| named == own),
            _ => false,
        }
    }
}

impl Drop for ScratchRoots {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}
