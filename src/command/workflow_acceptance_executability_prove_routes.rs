use super::*;

pub(super) fn alias_only(candidates: &[mutation::CandidateDiagnostic]) -> Option<String> {
    use mutation::ExclusionReason::*;
    let aliases: Vec<_> = candidates
        .iter()
        .filter(|c| c.exclusion == Some(RepositoryAliasUnverified))
        .map(|c| c.path.as_str())
        .collect();
    (!aliases.is_empty()
        && candidates.iter().all(|c| {
            matches!(
                c.exclusion,
                Some(
                    CheckMechanics
                        | RepositoryRoot
                        | WholeDirectoryTree
                        | RepositoryAliasUnverified
                )
            )
        }))
    .then(|| {
        format!(
            "host could not verify the repository alias: {}",
            aliases.join(", ")
        )
    })
}

pub(super) fn no_moves(
    probe: &HostProbe,
    baseline: &Baseline,
    contract: &AcceptanceContract,
    id: &str,
    short: &str,
    states: &BTreeMap<String, String>,
    lead: &str,
) -> Option<String> {
    if states
        .values()
        .any(|state| state.starts_with("move failed:"))
    {
        let why = format!("the host could not move the check's inputs: {states:?}");
        // A second identical move failure is permanently the host's, never
        // an author finding. Persist the strike using the original text.
        if super::super::silent::strike_mutation(
            probe,
            &baseline.commit,
            contract,
            id,
            "move",
            (&why, UnprovenCause::Host),
            || (why.clone(), why.clone()),
        )
        .is_some()
        {
            probe.unproven_as(id, why, UnprovenCause::PermanentHost);
        }
        return None;
    }
    if states.is_empty() {
        probe.unproven_as(
            id,
            "the host recorded no move states".into(),
            UnprovenCause::PermanentHost,
        );
        return None;
    }
    let commit_kind = if baseline.origin == BaselineOrigin::RecordedBase {
        "pre-implementation commit"
    } else {
        "selected commit"
    };
    let clauses: Vec<_> = states.iter().filter_map(|(path, state)| match state.as_str() {
        "absent at the selected root" => Some(format!("`{path}` does not exist in the {commit_kind} {short}; name a file that exists there")),
        "refused: symlink component" => Some(format!("`{path}` reaches its target through a symbolic link; name the real path")),
        _ => None,
    }).collect();
    Some(format!("{lead} {}", clauses.join("; ")))
}

pub(super) fn bound_accepted(
    entry: Option<&AcceptanceCriterion>,
    refs: &[FrozenCommandRef],
    digest: &str,
    id: &str,
) -> bool {
    entry.is_some_and(|entry| {
        entry.judgment.verdict == JudgeDecision::Accepted
            && refs.iter().any(|reference| {
                reference.acceptance_id == id
                    && reference.chain_digest == digest
                    && executed_text(entry).is_some_and(|(_, text)| {
                        reference.command_digest == content_digest(text.as_bytes())
                    })
            })
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn no_moves_for_result(
    probe: &HostProbe,
    baseline: &Baseline,
    contract: &AcceptanceContract,
    id: &str,
    short: &str,
    result: Option<&CheckEvidence>,
    markers: &mutation::Markers,
    names: &[String],
    lead: &str,
) -> Option<Option<String>> {
    let result = result?;
    if result.operational_error.is_some()
        || markers.guarded(&result.result)
        || markers.status(&result.result).is_none()
        || markers.observed_signal(&result.result).is_some()
        || !markers.moved(&result.result, names).is_empty()
    {
        return None;
    }
    let mut states = markers.move_states(&result.result, names);
    for name in names {
        states
            .entry(name.clone())
            .or_insert_with(|| "move failed: no move state was recorded".into());
    }
    let finding = no_moves(probe, baseline, contract, id, short, &states, lead).map(|finding| {
        format!(
            "{finding}; considered inputs: {}",
            super::describe_states(&states)
        )
    });
    Some(finding.map(|finding| {
        observed(
            finding,
            &mutation_observation(&result.result, markers, names, &probe.output_redactor()),
        )
    }))
}

/// Rebuild the host's copy layout once before blaming an unverified alias.
/// An author-fixable exclusion needs no host retry.
pub(super) async fn discover_with_rebuild<F, Fut>(
    entry: &AcceptanceCriterion,
    project: &Path,
    repository: &Path,
    layout: Option<&super::super::hermetic::CopyLayout>,
    rebuild: F,
) -> mutation::InputDiscovery
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<super::super::hermetic::CopyLayout>>,
{
    let discovery = mutation::input_discovery_for_check(entry, project, repository, layout);
    if !discovery.eligible.is_empty() || alias_only(&discovery.candidates).is_none() {
        return discovery;
    }
    match rebuild().await {
        Some(layout) => {
            mutation::input_discovery_for_check(entry, project, repository, Some(&layout))
        }
        None => discovery,
    }
}
