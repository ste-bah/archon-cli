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
            // Git lists each worktree by its resolved path: this site's copy
            // is compared by its own resolved identity, taken before git
            // removes it (an aliased root never reads as unregistered).
            let owned = resolved(&self.repository).ok_or_else(|| {
                invalid(format!(
                    "cannot resolve the owned scratch worktree {}; its registration is unverifiable",
                    self.repository.display()
                ))
            })?;
            // Whether the live repository's own administrative entry for this
            // copy is intact before git removes it: only then can a failed
            // removal that leaves the copy unregistered be git's own
            // unregistering (#534). An entry already gone or damaged is damage
            // outside the site (H5), never a teardown this host verified.
            let intact = self.links_own_worktree(&self.repository.join(".git"));
            let remove = || {
                git(
                    &self.live_repository,
                    &["worktree", "remove", "--force"],
                    &[&self.repository],
                )
            };
            #[cfg(test)]
            let removal = io::remove_tests::worktree_remove(&self.repository, remove);
            #[cfg(not(test))]
            let removal = remove();
            if let Err(error) = removal {
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
                        self.registered = registered_in(&listed, &self.repository, &owned);
                    }
                    if !self.registered {
                        return Err(WorkflowError::ScratchSiteRemoval(text));
                    }
                }
                // #534: a writer outside this host can add an entry to the
                // copy while git deletes it. Git still unregisters the
                // worktree; the owned removal below takes what is left.
                // Only when git reached that deletion: its own C-locale
                // failure for this copy (`git` runs with LC_ALL=C). A lookup
                // or validation refusal ("not a working tree") never did,
                // whatever the listing shows afterwards.
                if !intact || !raced_copy_delete(&text, &self.repository, &owned) {
                    return Err(error);
                }
                let listed = git(
                    &self.live_repository,
                    &["worktree", "list", "--porcelain"],
                    &[],
                );
                if !listed.is_ok_and(|listed| !registered_in(&listed, &self.repository, &owned)) {
                    return Err(error);
                }
            }
            let listed = git(
                &self.live_repository,
                &["worktree", "list", "--porcelain"],
                &[],
            )?;
            if registered_in(&listed, &self.repository, &owned) {
                return Err(invalid(
                    "owned scratch worktree remains registered after removal",
                ));
            }
            self.registered = false;
        }
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

/// Where `path` resolves: its canonical spelling, or, once it is gone, its
/// parent's canonical spelling with its name. `None` when neither resolves.
fn resolved(path: &Path) -> Option<PathBuf> {
    let canonical = |path: &Path| std::fs::canonicalize(path).map(archon_shell::paths::plain);
    if let Ok(path) = canonical(path) {
        return Some(path);
    }
    Some(canonical(path.parent()?).ok()?.join(path.file_name()?))
}

/// Whether git's failure is the one `git worktree remove` gives after it found
/// the worktree and began deleting the copy at `path` (resolved `owned`), and
/// a racing writer kept a directory non-empty (builtin/worktree.c
/// `delete_git_work_tree`: "failed to delete '<path>'" with the errno text).
/// The named path is compared as text with the copy's spellings taken before
/// git ran, never resolved: after the failure a writer may have made any
/// path (git's administrative entry included) a link to a recreated copy.
fn raced_copy_delete(text: &str, path: &Path, owned: &Path) -> bool {
    text.lines().any(|line| {
        let Some((_, rest)) = line.split_once("failed to delete '") else {
            return false;
        };
        let Some(named) = rest.trim_end().strip_suffix("': Directory not empty") else {
            return false;
        };
        let named = Path::new(named);
        named == path || named == owned
    })
}

/// Whether `git worktree list --porcelain` output still registers the copy
/// at `path`, whose resolved identity is `owned`. A listed path that no
/// longer resolves counts when it is spelled as `path`.
fn registered_in(listed: &str, path: &Path, owned: &Path) -> bool {
    listed
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(Path::new)
        .any(|named| named == path || resolved(named).is_some_and(|named| named == owned))
}

impl Drop for ScratchRoots {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}
