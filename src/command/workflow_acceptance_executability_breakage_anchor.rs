//! Proof execution against an explicit immutable tree and pair selection.
use super::*;
use archon_workflow::base_snapshot::{BaseSnapshot, SnapshotKind};
pub(crate) struct AnchoredPair {
    pub task_id: String,
    pub id: String,
    pub checks: Vec<String>,
    pub countable: Vec<(String, SnapshotKind)>,
    pub shared: Vec<(String, SnapshotKind)>,
    pub state: Option<ProofState>,
}
impl AnchoredPair {
    pub(super) fn proof(self) -> RequirementProof {
        RequirementProof {
            task_id: self.task_id,
            id: self.id,
            checks: self.checks,
            state: self.state.unwrap_or(ProofState::NoViableMutant),
            counted_kills: Vec::new(),
        }
    }
}
pub(crate) struct Anchor<'a> {
    pub contract: &'a AcceptanceContract,
    pub chain_digest: &'a str,
    pub commit: &'a str,
    pub snapshot: &'a BaseSnapshot,
    pub pairs: Vec<AnchoredPair>,
    pub mutants: Vec<SuppliedMutant>,
    pub max_runs: u64,
    pub min_deadline_secs: u64,
    pub store_root: Option<PathBuf>,
}
pub(crate) struct AnchoredReport {
    pub controls: BTreeMap<String, ControlRecord>,
    pub runs: BTreeMap<String, MutantRun>,
    pub entries: Vec<RequirementProof>,
    pub refusals: Vec<String>,
    pub runs_executed: u64,
    pub runs_reused: u64,
}
/// Callers must not attach a baseline to an unavailable execution site.
pub(crate) async fn prove_anchored(
    probe: &HostProbe,
    mut anchor: Anchor<'_>,
    mut refusals: Vec<String>,
) -> AnchoredReport {
    let prior_len = refusals.len();
    if matches!(probe.site, Site::Scratch(_)) {
        refusals.push("proof_site_unavailable".into());
    }
    if probe.baseline_commit().as_deref() != Some(anchor.commit) {
        refusals.push("probe_baseline_mismatch".into());
    }
    if !anchor.snapshot.valid_digest() || anchor.snapshot.commit != anchor.commit {
        refusals.push("snapshot_invalid".into());
    }
    let pairs = anchor
        .pairs
        .iter()
        .map(|p| (p.task_id.clone(), p.id.clone()))
        .collect::<Vec<_>>();
    let set = MutantSet {
        schema: BREAKAGE_SCHEMA,
        base_commit: anchor.commit.into(),
        mutants: anchor.mutants,
    };
    let (mutants, codes) =
        selection::validate(&set, anchor.commit, &pairs).expect("current schema and commit");
    anchor.mutants = mutants;
    for pair in &mut anchor.pairs {
        if pair.state.is_none() {
            pair.state = if pair.checks.is_empty() {
                Some(ProofState::NoPassingCheck)
            } else if pair.countable.is_empty() {
                Some(ProofState::NoCountableDeliverable)
            } else if !anchor
                .mutants
                .iter()
                .any(|m| m.task_id == pair.task_id && m.requirement_id == pair.id)
            {
                Some(ProofState::NoMutants)
            } else {
                None
            };
        }
        if refusals.len() > prior_len
            && (pair.state.is_none() || pair.state == Some(ProofState::NoMutants))
        {
            pair.state = Some(ProofState::HostUnrun);
        }
    }
    refusals.extend(codes);
    execute(probe, anchor, refusals).await
}
pub(super) async fn sweep_copies(probe: &HostProbe, refusals: &mut Vec<String>) {
    let parent = probe.copy_parent.clone();
    match run::blocking(move || Ok(sweep(&parent))).await {
        Ok(failures) => refusals.extend(failures),
        Err(e) => refusals.push(format!("{e:?}")),
    }
}
async fn prepare(probe: &HostProbe, work: &Path, commit: &str) -> Result<(), RunOutcome> {
    let (parent, work, repository, commit) = (
        probe.copy_parent.clone(),
        work.to_path_buf(),
        probe.repository.clone(),
        commit.to_string(),
    );
    run::blocking(move || {
        base_copy::baseline_available(&repository, &commit).map_err(patch::host)?;
        std::fs::create_dir_all(&parent).map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        owner::write(&work).map_err(patch::host)?;
        std::fs::create_dir_all(work.join("patches"))
            .map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        std::fs::create_dir_all(work.join("build-output"))
            .map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        let git = base_git::IsolatedGit::new(&work).map_err(patch::host)?;
        let transport = work.join("base.git");
        let format = base_copy::output(&repository, &["rev-parse", "--show-object-format"])
            .map_err(patch::host)?;
        let format = format!("--object-format={}", format.trim());
        git.run(&[
            "init".as_ref(),
            "--bare".as_ref(),
            "--quiet".as_ref(),
            format.as_ref(),
            transport.as_os_str(),
        ])
        .map_err(patch::host)?;
        let top = base_copy::output(&repository, &["rev-parse", "--show-toplevel"])
            .map_err(patch::host)?;
        let result = git
            .output(&[
                "-C".as_ref(),
                transport.as_os_str(),
                "fetch".as_ref(),
                "--no-tags".as_ref(),
                "--depth=1".as_ref(),
                top.trim().as_ref(),
                commit.as_ref(),
            ])
            .map_err(patch::host)?;
        if !result.status.success() {
            return Err(decide::unrun(
                "baseline_fetch_failed",
                &String::from_utf8_lossy(&result.stderr),
            ));
        }
        Ok(())
    })
    .await
}

pub(super) async fn execute(
    probe: &HostProbe,
    anchor: Anchor<'_>,
    prior: Vec<String>,
) -> AnchoredReport {
    let mut proofs = BreakageProofs {
        schema: BREAKAGE_SCHEMA,
        base_commit: anchor.commit.into(),
        controls: BTreeMap::new(),
        runs: BTreeMap::new(),
        summary: None,
    };
    let mut summary = ProofSummary {
        acceptance_digest: String::new(),
        baseline_results_digest: String::new(),
        snapshot_digest: anchor.snapshot.digest.clone(),
        runs_executed: 0,
        runs_reused: 0,
        refusals: prior,
        entries: Vec::new(),
    };
    let mut groups: BTreeMap<String, Vec<SuppliedMutant>> = BTreeMap::new();
    for pair in &anchor.pairs {
        if pair.state.is_none() {
            for id in &pair.checks {
                groups.entry(id.clone()).or_default().extend(
                    anchor
                        .mutants
                        .iter()
                        .filter(|m| m.task_id == pair.task_id && m.requirement_id == pair.id)
                        .cloned(),
                );
            }
        }
        summary.entries.push(RequirementProof {
            task_id: pair.task_id.clone(),
            id: pair.id.clone(),
            checks: pair.checks.clone(),
            state: pair.state.unwrap_or(ProofState::NoViableMutant),
            counted_kills: Vec::new(),
        });
    }
    sweep_copies(probe, &mut summary.refusals).await;
    if !groups.is_empty() {
        let work = probe
            .copy_parent
            .join(format!("{COPY_PREFIX}breakage-{}", uuid::Uuid::new_v4()));
        let prepared = prepare(probe, &work, anchor.commit).await;
        for (id, mut mutants) in groups {
            mutants.sort_by_key(|m| (m.task_id.clone(), m.requirement_id.clone(), m.mutant_id()));
            group::execute(
                probe,
                &anchor,
                &id,
                mutants,
                &work,
                anchor.store_root.as_deref(),
                &prepared,
                &mut proofs,
                &mut summary,
            )
            .await;
        }
        let remove = work.clone();
        if let Err(e) =
            run::blocking(move || owner::remove(&remove).map_err(|e| decide::unrun("host_io", &e)))
                .await
        {
            summary
                .refusals
                .push(format!("work_dir_not_removed: {} ({e:?})", work.display()));
        }
    }
    for pair in &mut summary.entries {
        if pair.state != ProofState::NoViableMutant {
            continue;
        }
        let runs: Vec<_> = proofs
            .runs
            .values()
            .filter(|r| r.task_id == pair.task_id && r.requirement_id == pair.id)
            .collect();
        pair.counted_kills = runs
            .iter()
            .filter(|r| r.counted)
            .map(|r| r.mutant_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        pair.state = if runs
            .iter()
            .any(|r| r.counted && r.kind == MutantKind::Behaviour)
        {
            ProofState::BehaviourKill
        } else if runs.iter().any(|r| r.counted) {
            ProofState::BodyGutKillOnly
        } else if runs
            .iter()
            .any(|r| matches!(r.outcome, RunOutcome::BudgetExhausted))
        {
            ProofState::BudgetExhausted
        } else if runs.iter().any(|r| {
            matches!(
                r.outcome,
                RunOutcome::Unrun { ref code, .. } if code != "control_mismatch" && code != "zero_work"
            ) || matches!(r.outcome, RunOutcome::NoExitStatus)
        }) || proofs.controls.values().any(|c| {
            pair.checks.contains(&c.check_id)
                && [&c.pre, c.post.as_ref().unwrap_or(&c.pre)]
                    .into_iter()
                    .any(|o| {
                        matches!(o, RunOutcome::Unrun { code, .. } if code != "control_mismatch" && code != "zero_work")
                            || matches!(o, RunOutcome::NoExitStatus)
                    })
        }) {
            ProofState::HostUnrun
        } else if pair.checks.iter().all(|c| {
            !proofs
                .controls
                .values()
                .any(|r| &r.check_id == c && r.stable())
        }) {
            ProofState::Unverified
        } else if runs.iter().any(|r| {
            matches!(r.outcome, RunOutcome::Survived { .. })
                && !runs.iter().any(|other| {
                    other.mutant_id == r.mutant_id
                        && matches!(other.outcome, RunOutcome::Killed { .. })
                })
        }) {
            ProofState::SurvivorsOnly
        } else {
            ProofState::NoViableMutant
        };
    }

    AnchoredReport {
        controls: proofs.controls,
        runs: proofs.runs,
        entries: summary.entries,
        refusals: summary.refusals,
        runs_executed: summary.runs_executed,
        runs_reused: summary.runs_reused,
    }
}
