//! Everything a per-check repair verifies before any model is asked, and
//! re-verifies immediately before it publishes.

use archon_workflow::task_set_contract::{
    ACCEPTANCE_LOCK_FILE, FreezeGateMode, TASK_SKELETON_LOCK_FILE,
};
use archon_workflow::task_skeleton::validate_full_chain;

use super::*;

/// Everything verified about the chain before any model is asked.
pub(super) struct Verified {
    pub(super) pin: AcceptancePin,
    pub(super) pin_path: PathBuf,
    pub(super) contract: AcceptanceContract,
    pub(super) canonical_prd: PathBuf,
    pub(super) prd_text: String,
    pub(super) expected: BTreeSet<String>,
    pub(super) skeleton: Option<TaskSkeleton>,
    /// The gate mode each stage of this chain was frozen in.
    pub(super) acceptance_mode: FreezeGateMode,
    pub(super) skeleton_mode: FreezeGateMode,
    /// The (model, provider) the freeze-time judge recorded on every check;
    /// the repair re-judges with exactly that judge.
    pub(super) judge: (String, String),
    /// blake3 of every chain file as verified (`None` = absent).
    digests: Vec<(PathBuf, Option<String>)>,
}

fn chain_files(tasks_root: &Path, pin_path: &Path) -> [PathBuf; 5] {
    [
        tasks_root.join(ACCEPTANCE_CONTRACT_FILE),
        tasks_root.join(ACCEPTANCE_LOCK_FILE),
        tasks_root.join(TASK_SKELETON_FILE),
        tasks_root.join(TASK_SKELETON_LOCK_FILE),
        pin_path.to_path_buf(),
    ]
}

fn digests(files: &[PathBuf]) -> Vec<(PathBuf, Option<String>)> {
    files
        .iter()
        .map(|path| {
            (
                path.clone(),
                std::fs::read(path).ok().map(|bytes| content_digest(&bytes)),
            )
        })
        .collect()
}

impl Verified {
    /// The digest every chain file held when verified (`None` = absent);
    /// the publish transaction refuses to replace a file that moved since.
    pub(super) fn prior(&self) -> &[(PathBuf, Option<String>)] {
        &self.digests
    }
}

/// The freeze-time judge: the one (model, provider) every judged check
/// records. A per-check repair is judged by exactly that judge, so a contract
/// that records none, or more than one, cannot be repaired per check.
fn recorded_judge(contract: &AcceptanceContract) -> Result<(String, String)> {
    let mut judges = BTreeSet::new();
    let mut unrecorded = Vec::new();
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        let recorded = entry.judgment.sampling.as_ref().and_then(|sampling| {
            Some((
                sampling["model"].as_str()?.to_string(),
                sampling["provider"].as_str()?.to_string(),
            ))
        });
        match recorded {
            Some(judge) => {
                judges.insert(judge);
            }
            None => unrecorded.push(entry.id.clone()),
        }
    }
    if !unrecorded.is_empty() {
        return Err(anyhow!(
            "the frozen contract records no judge model and provider for {}; a per-check repair must be judged by the freeze-time judge, so re-run the whole-set freeze-acceptance",
            unrecorded.join(", ")
        ));
    }
    let mut judges = judges.into_iter();
    match (judges.next(), judges.next()) {
        (Some(judge), None) => Ok(judge),
        (Some(first), Some(second)) => Err(anyhow!(
            "the frozen contract was judged by more than one judge ({}/{} and {}/{} at least); a per-check repair cannot match them all, so re-run the whole-set freeze-acceptance",
            first.0,
            first.1,
            second.0,
            second.1
        )),
        (None, _) => Err(anyhow!("the frozen contract holds no judged check")),
    }
}

pub(super) fn verify(request: &ReauthorRequest<'_>) -> Result<Verified> {
    verify_with(request, None)
}

/// [`verify`] for an extension (ACC-A7): `adding` are the ids it adds or
/// re-authors -- exactly the request's ids, new to the contract or held by
/// it -- and the PRD may have moved since the freeze: the chain is verified
/// against the ids it was frozen with, and the republished one against the
/// PRD as it is now.
pub(super) fn verify_with(
    request: &ReauthorRequest<'_>,
    adding: Option<&BTreeSet<String>>,
) -> Result<Verified> {
    let (project_root, tasks_root) = (request.project_root, request.tasks_root);
    let pin_path = acceptance_pin_path(project_root, tasks_root);
    let digests = digests(&chain_files(tasks_root, &pin_path));
    let pin: AcceptancePin = serde_json::from_slice(&std::fs::read(&pin_path).with_context(
        || {
            format!(
                "acceptance pin {} could not be read; --reauthor repairs a frozen contract, so run the whole-set freeze-acceptance first (a crashed publish leaves the prior pin as a .old backup beside it)",
                pin_path.display()
            )
        },
    )?)
    .with_context(|| format!("acceptance pin {} is malformed", pin_path.display()))?;
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let bytes = std::fs::read(&contract_path)
        .with_context(|| format!("reading {}", contract_path.display()))?;
    let contract: AcceptanceContract = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {}", contract_path.display()))?;
    // Byte identity of every entry not named rests on this: the file is the
    // host's own serialization, so re-serializing an entry reproduces it.
    if serde_json::to_vec_pretty(&contract)? != bytes {
        return Err(anyhow!(
            "{} is not in the host's canonical serialization, so the entries not named could not be kept byte-identical; restore the frozen file or re-run the whole-set freeze-acceptance",
            contract_path.display()
        ));
    }
    let canonical_prd = request
        .prd_path
        .canonicalize()
        .map(archon_shell::paths::plain)
        .with_context(|| format!("canonicalizing PRD {}", request.prd_path.display()))?;
    let contracted = PathBuf::from(&contract.prd.path);
    let contracted = if contracted.is_absolute() {
        contracted
    } else {
        project_root.join(contracted)
    };
    if contracted
        .canonicalize()
        .map(archon_shell::paths::plain)
        .ok()
        .as_deref()
        != Some(canonical_prd.as_path())
    {
        return Err(anyhow!(
            "--prd {} is not the contract's frozen PRD {}; pass the frozen PRD",
            canonical_prd.display(),
            contracted.display()
        ));
    }
    let prd = std::fs::read(&canonical_prd)?;
    if adding.is_none() && content_digest(&prd) != contract.prd.digest {
        return Err(anyhow!(
            "PRD {} changed since the contract was frozen (digest {} != {}); a changed PRD needs the whole-set freeze-acceptance, not a per-check repair",
            canonical_prd.display(),
            content_digest(&prd),
            contract.prd.digest
        ));
    }
    let prd_text = String::from_utf8(prd).context("frozen PRD is not UTF-8")?;
    let expected: BTreeSet<String> = acceptance_criteria(&prd_text).into_keys().collect();
    let frozen_ids: BTreeSet<String> = match adding {
        Some(_) => contract.acceptance.iter().map(|e| e.id.clone()).collect(),
        None => expected.clone(),
    };
    validate_acceptance_bundle(tasks_root, Some(&pin), &frozen_ids)
        .map_err(|error| anyhow!("the frozen acceptance chain does not verify: {error}"))?;
    let known = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect::<BTreeSet<_>>();
    if request.ids.is_empty() {
        return Err(anyhow!("--reauthor needs at least one check id"));
    }
    if adding.is_some_and(|adding| adding != request.ids) {
        return Err(anyhow!(
            "an extension names exactly the checks it adds or re-authors"
        ));
    }
    let unknown = match adding {
        Some(_) => Vec::new(),
        None => request.ids.difference(&known).cloned().collect::<Vec<_>>(),
    };
    if !unknown.is_empty() {
        return Err(anyhow!(
            "--reauthor names check(s) not in the frozen contract: {}; the contract holds {}",
            unknown.join(", "),
            known.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let unnamed = non_accepted_ids(&contract)
        .difference(request.ids)
        .cloned()
        .collect::<Vec<_>>();
    if !unnamed.is_empty() {
        return Err(anyhow!(
            "the contract also carries check(s) the judge did not accept: {}; name each with --reauthor so the republished contract holds none",
            unnamed.join(", ")
        ));
    }
    let skeleton = match (
        tasks_root.join(TASK_SKELETON_FILE).exists(),
        tasks_root.join(TASK_SKELETON_LOCK_FILE).exists(),
        pin.skeleton_digest.is_some() || pin.skeleton_gate.is_some(),
    ) {
        (false, false, false) => None,
        (true, true, true) => Some(validate_full_chain(tasks_root, &pin).map_err(|error| {
            anyhow!(
                "the frozen skeleton chain does not verify: {error}; run workflow freeze-skeleton to restore it before a per-check repair"
            )
        })?),
        (file, lock, pinned) => {
            return Err(anyhow!(
                "partial skeleton freeze beside {}: file={file}, lock={lock}, pin={pinned}; run workflow freeze-skeleton first",
                tasks_root.display()
            ));
        }
    };
    if let Some(skeleton) = &skeleton {
        // The skeleton is rewritten with only its acceptance digest moved;
        // that is byte-identity only if the file is the host's serialization.
        let on_disk = std::fs::read(tasks_root.join(TASK_SKELETON_FILE))?;
        if serde_json::to_vec_pretty(skeleton)? != on_disk {
            return Err(anyhow!(
                "{} is not in the host's canonical serialization, so re-binding it would change more than its acceptance digest; run workflow freeze-skeleton instead",
                tasks_root.join(TASK_SKELETON_FILE).display()
            ));
        }
    }
    let skeleton_mode = pin
        .skeleton_gate
        .as_ref()
        .map_or(pin.acceptance_gate.mode, |gate| gate.mode);
    Ok(Verified {
        acceptance_mode: pin.acceptance_gate.mode,
        skeleton_mode,
        judge: recorded_judge(&contract)?,
        pin,
        pin_path,
        contract,
        canonical_prd,
        prd_text,
        expected,
        skeleton,
        digests,
    })
}
