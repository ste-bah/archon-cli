//! The pre-implementation probe: every check must be able to fail (A4, A5).
//!
//! The judge weighs a check; it never runs one, so a check that passes
//! whatever the implementation does was published and then "passed". Here a
//! probe given a [`Baseline`] runs each check that ran soundly on the tree
//! BEFORE any implementation and requires it to fail there:
//!
//! - it passed there (exit 0 with real work): a finding for its author --
//!   it cannot show its criterion false, so it proves nothing;
//! - the host could not run it there (after one retry): a finding too -- it
//!   is not yet proven able to fail, so it is probed again before it may be
//!   published, never published unproven;
//! - it failed there: sound.
//!
//! The baseline runs only in a hermetic copy, never the live tree: the
//! scratch site's own observation at the baseline commit, or, without a
//! scratch policy, a temporary clone of the repository at that commit (the
//! project is the clone's copy of it, so a project outside the repository
//! has no hermetic copy: that is recorded as a diagnostic, and the check is
//! not held back for a site the project never configured).

use std::path::{Path, PathBuf};

use super::*;

/// The tree before any implementation, in the repository it belongs to.
#[derive(Debug, Clone)]
pub(crate) struct Baseline {
    pub(crate) commit: String,
    pub(crate) repository: PathBuf,
}

impl Baseline {
    /// The tree a freeze stands on: `repository`'s HEAD. `None` when it is
    /// not a git checkout.
    pub(crate) fn head_of(repository: &Path) -> Option<Self> {
        Some(Self {
            commit: git_head(repository)?,
            repository: repository.to_path_buf(),
        })
    }
}

/// Why the baseline gave no verdict.
enum Unrun {
    /// No hermetic copy can be formed for this project: a diagnostic.
    NoSite(String),
    /// The copy or the run failed: probed again, a finding if it persists.
    Failed(String),
}

/// Author findings, keyed by id, for every check of `refs` that passed on
/// the baseline or could not be run there (see the module docs). `known`
/// holds verdicts already observed on the baseline tree itself.
pub(super) async fn cannot_fail_findings(
    probe: &HostProbe,
    baseline: &Baseline,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    known: Option<&[CheckResult]>,
) -> BTreeMap<String, String> {
    let mut findings = BTreeMap::new();
    if refs.is_empty() {
        return findings;
    }
    let short: String = baseline.commit.chars().take(12).collect();
    // Verdicts already observed on this very tree are not observed twice.
    let mut results: BTreeMap<String, CheckResult> = (known.into_iter().flatten())
        .filter(|result| result.operational_error.is_none())
        .map(|result| (result.acceptance_id.clone(), result.clone()))
        .collect();
    let mut why = String::new();
    // One retry for whatever the first attempt could not run.
    for _ in 0..2 {
        let pending: Vec<FrozenCommandRef> = (refs.iter())
            .filter(|reference| {
                (results.get(&reference.acceptance_id))
                    .is_none_or(|result| result.operational_error.is_some())
            })
            .cloned()
            .collect();
        if pending.is_empty() {
            break;
        }
        match run_at(probe, baseline, contract, digest, &pending).await {
            Ok(ran) => {
                for result in ran {
                    results.insert(result.acceptance_id.clone(), result);
                }
            }
            Err(Unrun::NoSite(reason)) => {
                probe.note(format!(
                    "pre-implementation probe not run at {short}: {reason}; check(s) {} are not proven able to fail",
                    pending
                        .iter()
                        .map(|r| r.acceptance_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                return findings;
            }
            Err(Unrun::Failed(reason)) => why = reason,
        }
    }
    for reference in refs {
        let id = &reference.acceptance_id;
        match results.get(id) {
            Some(result) if result.operational_error.is_none() => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let stderr = String::from_utf8_lossy(&result.stderr);
                let passed = result.exit_code == Some(0)
                    && !archon_workflow::acceptance::output_reports_zero_work(&stdout, &stderr);
                if passed {
                    findings.insert(
                        id.clone(),
                        format!(
                            "check '{id}': it passed on the pre-implementation tree at {short}, before any implementation, so it cannot show its criterion false and proves nothing; make it exercise what the implementation must add, so that it fails on that tree and passes only once the criterion holds"
                        ),
                    );
                }
            }
            other => {
                let reason = other
                    .and_then(|result| result.operational_error.clone())
                    .unwrap_or_else(|| why.clone());
                findings.insert(
                    id.clone(),
                    format!(
                        "check '{id}': the host could not run it on the pre-implementation tree at {short} ({reason}), so it is not yet proven able to fail; it is probed again before it may be published"
                    ),
                );
            }
        }
    }
    findings
}

async fn run_at(
    probe: &HostProbe,
    baseline: &Baseline,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
) -> Result<Vec<CheckResult>, Unrun> {
    if let Site::Scratch(binding) = &probe.site {
        let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
        return tokio::spawn(observe(
            binding.clone(),
            Some(baseline.commit.clone()),
            contract.clone(),
            digest.to_string(),
            refs.to_vec(),
            cancel.0.clone(),
        ))
        .await
        .map_err(|error| Unrun::Failed(format!("scratch probe task failed: {error}")))?
        .map_err(|error| Unrun::Failed(format!("{error:#}")));
    }
    let clone = TempClone::at(&baseline.repository, &probe.project, &baseline.commit)?;
    let site = DirectSite {
        repository: clone.repository.clone(),
        project: clone.project.clone(),
        environment: archon_tools::bash::host_env().into_iter().collect(),
        timeout_secs: DIRECT_DEFAULT_TIMEOUT_SECS,
        output_bytes: DIRECT_DEFAULT_OUTPUT_BYTES,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut results = Vec::new();
    for reference in refs {
        match run_check_direct(&site, contract, digest, reference, cancel.clone()).await {
            Ok(result) => results.push(result),
            Err(error) => return Err(Unrun::Failed(error.to_string())),
        }
    }
    Ok(results)
}

/// A clone of the repository at one commit under the system temporary
/// directory, removed when dropped. It shares the source's objects and
/// writes nothing into the source.
struct TempClone {
    root: PathBuf,
    repository: PathBuf,
    project: PathBuf,
}

impl TempClone {
    fn at(repository: &Path, project: &Path, commit: &str) -> Result<Self, Unrun> {
        let canonical = |path: &Path| {
            path.canonicalize()
                .map_err(|error| Unrun::Failed(format!("{}: {error}", path.display())))
        };
        let (source, project) = (canonical(repository)?, canonical(project)?);
        let Ok(relative) = project.strip_prefix(&source).map(Path::to_path_buf) else {
            return Err(Unrun::NoSite(format!(
                "no [workflow.acceptance_execution] scratch policy is configured and the project root {} lies outside the repository {}, so no hermetic copy of both can be formed",
                project.display(),
                source.display()
            )));
        };
        let root = std::env::temp_dir().join(format!("archon-baseline-{}", uuid::Uuid::new_v4()));
        let clone = Self {
            repository: root.join("repo"),
            project: root.join("repo").join(relative),
            root,
        };
        let git = |args: &[&std::ffi::OsStr]| {
            let output = std::process::Command::new("git")
                .args(args)
                .output()
                .map_err(|error| Unrun::Failed(format!("git: {error}")))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(Unrun::Failed(format!(
                    "git {}: {}",
                    args.iter()
                        .map(|arg| arg.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(" "),
                    String::from_utf8_lossy(&output.stderr).trim()
                )))
            }
        };
        git(&[
            "clone".as_ref(),
            "--quiet".as_ref(),
            "--shared".as_ref(),
            "--no-checkout".as_ref(),
            source.as_os_str(),
            clone.repository.as_os_str(),
        ])?;
        git(&[
            "-C".as_ref(),
            clone.repository.as_os_str(),
            "checkout".as_ref(),
            "--quiet".as_ref(),
            "--detach".as_ref(),
            commit.as_ref(),
        ])?;
        if !clone.project.is_dir() {
            return Err(Unrun::NoSite(format!(
                "the project root is not tracked at {commit}, so the clone holds no copy of it"
            )));
        }
        Ok(clone)
    }
}

impl Drop for TempClone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
