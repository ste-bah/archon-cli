//! The host probe itself: run each check at the site, then hold it to the
//! trees it must be proven on (see the parent module docs).

use super::*;

impl HostProbe {
    /// The commit the site observes, when it observes one.
    pub(super) fn site_commit(&self) -> Option<String> {
        match &self.site {
            Site::Scratch(binding) => git_head(&binding.policy.repository),
            Site::Hermetic => git_head(&self.repository),
            Site::Direct | Site::Unavailable(_) => None,
        }
    }

    /// The live roots, canonical where they resolve, as check text could
    /// name them.
    pub(super) fn live_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for root in [&self.repository, &self.project] {
            roots.push(root.clone());
            if let Ok(canonical) = root.canonicalize().map(archon_shell::paths::plain) {
                roots.push(canonical);
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }

    /// Author findings for every check of `refs` whose text names a live
    /// root by its absolute path: run in a copy, it would still read or
    /// change the live tree, so it is never run.
    fn live_root_findings(
        &self,
        contract: &AcceptanceContract,
        refs: &[FrozenCommandRef],
    ) -> BTreeMap<String, String> {
        let roots = self.live_roots();
        (refs.iter())
            .filter_map(|reference| {
                let entry = (contract.acceptance.iter())
                    .chain(&contract.supplementary)
                    .find(|entry| entry.id == reference.acceptance_id)?;
                let (_, text) = executed_text(entry)?;
                let named = roots.iter().find(|root| {
                    let root = root.to_string_lossy();
                    !root.is_empty() && root != "/" && text.contains(root.as_ref())
                })?;
                Some((
                    entry.id.clone(),
                    format!(
                        "check '{}': it names the live root {} by its absolute path, so no hermetic copy can keep it off the live tree and the host never runs it; name every path relative to the check's working directory",
                        entry.id,
                        named.display()
                    ),
                ))
            })
            .collect()
    }

    pub(super) async fn run(
        &self,
        contract: &AcceptanceContract,
        digest: &str,
        refs: &[FrozenCommandRef],
    ) -> Vec<CheckResult> {
        let ids = || {
            refs.iter()
                .map(|r| r.acceptance_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        match &self.site {
            Site::Unavailable(_) => return Vec::new(),
            Site::Direct => {
                let site = DirectSite {
                    repository: self.repository.clone(),
                    project: self.project.clone(),
                    // Exactly the round's own direct site environment.
                    environment: archon_tools::bash::host_env().into_iter().collect(),
                    // Issue 323: the probe's one per-check bound.
                    timeout_secs: self.check_bound_secs(),
                    output_bytes: DIRECT_DEFAULT_OUTPUT_BYTES,
                };
                let cancel = Arc::new(AtomicBool::new(false));
                let mut results = Vec::new();
                for reference in refs {
                    match run_check_direct(&site, contract, digest, reference, cancel.clone())
                        .await
                    {
                        Ok(result) => results.push(result),
                        Err(error) => self.note(format!(
                            "executability probe of '{}' could not run ({error}); it is proven on the hermetic trees instead",
                            reference.acceptance_id
                        )),
                    }
                }
                return results;
            }
            Site::Scratch(_) | Site::Hermetic => {}
        }
        let Some(head) = self.site_commit() else {
            self.note(format!(
                "executability probe not run: {} has no commit to copy; check(s) {} were not executed at the site",
                self.repository.display(),
                ids()
            ));
            return Vec::new();
        };
        let tree = Baseline {
            commit: head,
            repository: self.repository.clone(),
        };
        let run = repairs::tree_results(self, &tree, contract, digest, refs, None, false).await;
        (refs.iter())
            .filter_map(|reference| run.results.get(&reference.acceptance_id).cloned())
            .collect()
    }

    /// Run `ids` and hold them to every tree this probe proves on; with
    /// `hold`, first record how each fared at the site as its original.
    pub(super) async fn probe(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
        hold: bool,
    ) -> BTreeMap<String, String> {
        let digest = match contract_digest(contract) {
            Ok(digest) => digest,
            Err(error) => {
                self.note(format!(
                    "executability probe could not encode the contract: {error}"
                ));
                for id in ids {
                    self.unproven(id, format!("the contract could not be encoded: {error}"));
                }
                return BTreeMap::new();
            }
        };
        // Whatever this call proves replaces an earlier "unproven".
        (self.unproven.lock().expect("unproven lock")).retain(|id, _| !ids.contains(id));
        let all = refs_for(contract, &digest, ids);
        if all.is_empty() {
            return BTreeMap::new();
        }
        if let Site::Unavailable(reason) = &self.site {
            for reference in &all {
                self.unproven(&reference.acceptance_id, reason.clone());
            }
            return BTreeMap::new();
        }
        let mut findings = self.live_root_findings(contract, &all);
        let refs: Vec<FrozenCommandRef> = (all.into_iter())
            .filter(|reference| !findings.contains_key(&reference.acceptance_id))
            .collect();
        let results = if refs.is_empty() {
            Vec::new()
        } else {
            self.run(contract, &digest, &refs).await
        };
        if hold {
            let observed = originals(&self.check_site(contract), contract, results.clone()).await;
            let mut tree = self.failed_tree.lock().expect("failed tree lock");
            let held = tree.get_or_insert_with(|| FailedTree {
                commit: self.site_commit(),
                originals: BTreeMap::new(),
            });
            held.originals.extend(observed);
        }
        let binding = match &self.site {
            Site::Scratch(binding) => Some(binding.as_ref()),
            _ => None,
        };
        findings.extend(crash_findings_at(contract, &results, binding));
        // A4/A5: what did not crash must also be able to fail, and a repair
        // must keep its original's verdict. A check the site could not run
        // is still proven on the hermetic trees: it is never published on
        // the strength of a site failure.
        let sound: Vec<FrozenCommandRef> = (refs.iter())
            .filter(|reference| !findings.contains_key(&reference.acceptance_id))
            .cloned()
            .collect();
        let site_commit = self.site_commit();
        let on_site = |commit: &str| match &self.site {
            Site::Direct | Site::Unavailable(_) => false,
            Site::Scratch(_) | Site::Hermetic => site_commit.as_deref() == Some(commit),
        };
        let failed_tree = self.failed_tree.lock().expect("failed tree lock").clone();
        if let (false, Some(tree)) = (hold, &failed_tree) {
            // The round's own site is the tree its checks just failed on.
            let here = match (&self.site, &tree.commit) {
                (Site::Direct, _) | (_, None) => true,
                (_, Some(commit)) => on_site(commit),
            };
            findings.extend(
                baseline::newly_passing_findings(
                    self,
                    tree,
                    contract,
                    &digest,
                    &sound,
                    here.then_some(results.as_slice()),
                )
                .await,
            );
        }
        if self.baseline.is_none() {
            // Issue 275: a freeze with no pre-implementation tree has no
            // evidence that any check can fail, or can pass. The host's,
            // never published as proven.
            for reference in &sound {
                if !findings.contains_key(&reference.acceptance_id) {
                    self.unproven(
                        &reference.acceptance_id,
                        format!(
                            "there is no pre-implementation tree to run it on for {}, so it is not proven able to fail or able to pass",
                            self.repository.display()
                        ),
                    );
                }
            }
        }
        if let Some(baseline) = &self.baseline {
            // The site already observed its commit: when that commit is the
            // baseline (a freeze), its verdicts are the baseline's.
            let known = on_site(&baseline.commit).then_some(results.as_slice());
            let sound: Vec<FrozenCommandRef> = (sound.into_iter())
                .filter(|reference| !findings.contains_key(&reference.acceptance_id))
                .collect();
            findings.extend(
                baseline::cannot_fail_findings(self, baseline, contract, &digest, &sound, known)
                    .await,
            );
        }
        // A check proven unrunnable is the host's, never also the author's.
        let unproven = self.unproven.lock().expect("unproven lock").clone();
        findings.retain(|id, _| !unproven.contains_key(id));
        findings
    }
}

/// What [`HostProbe::prove_can_fail`] established for each check it was
/// asked about: proven able to fail, a finding for its author, or unproven
/// (the host's). A check with no script to run is in none of them.
#[derive(Debug, Default)]
pub(crate) struct CanFail {
    pub(crate) proven: BTreeSet<String>,
    pub(crate) findings: BTreeMap<String, String>,
    pub(crate) unproven: BTreeMap<String, String>,
}

impl HostProbe {
    /// Issue 219 (A5 at run time): hold `ids` -- checks a round already ran
    /// and saw pass -- to the baseline alone: each must fail on the
    /// pre-implementation tree, or fail there once the inputs it names are
    /// moved aside. Nothing runs at the site again (the round ran it there).
    pub(crate) async fn prove_can_fail(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> CanFail {
        let mut out = CanFail::default();
        let digest = match contract_digest(contract) {
            Ok(digest) => digest,
            Err(error) => {
                for id in ids {
                    let why = format!("the contract could not be encoded: {error}");
                    out.unproven.insert(id.clone(), why);
                }
                return out;
            }
        };
        (self.unproven.lock().expect("unproven lock")).retain(|id, _| !ids.contains(id));
        let all = refs_for(contract, &digest, ids);
        let mut findings = self.live_root_findings(contract, &all);
        let sound: Vec<FrozenCommandRef> = (all.iter())
            .filter(|reference| !findings.contains_key(&reference.acceptance_id))
            .cloned()
            .collect();
        match (&self.site, &self.baseline) {
            (Site::Unavailable(reason), _) => {
                for reference in &sound {
                    self.unproven(&reference.acceptance_id, reason.clone());
                }
            }
            (_, None) => {
                for reference in &sound {
                    self.unproven(
                        &reference.acceptance_id,
                        "the run has no pre-implementation tree to run it on, so it is not proven able to fail".to_string(),
                    );
                }
            }
            (_, Some(baseline)) => findings.extend(
                baseline::cannot_fail_findings(self, baseline, contract, &digest, &sound, None)
                    .await,
            ),
        }
        out.unproven = self.take_unproven();
        findings.retain(|id, _| !out.unproven.contains_key(id));
        out.proven = (all.into_iter())
            .map(|reference| reference.acceptance_id)
            .filter(|id| !findings.contains_key(id) && !out.unproven.contains_key(id))
            .collect();
        out.findings = findings;
        out
    }
}

#[async_trait]
impl ExecutabilityProbe for HostProbe {
    async fn script_defects(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        self.probe(contract, ids, false).await
    }

    async fn hold_originals(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        self.probe(contract, ids, true).await
    }

    fn take_diagnostics(&self) -> Vec<String> {
        std::mem::take(&mut *self.diagnostics.lock().expect("diagnostics lock"))
    }

    fn take_unproven(&self) -> BTreeMap<String, String> {
        std::mem::take(&mut *self.unproven.lock().expect("unproven lock"))
    }

    fn take_baseline_runs(&self) -> Option<BaselineRuns> {
        self.baseline_runs()
    }

    fn baseline_commit(&self) -> Option<String> {
        self.baseline
            .as_ref()
            .map(|baseline| baseline.commit.clone())
    }
}
