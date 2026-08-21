// Deciding a write item's true scope, at the moment its result is validated.
//
// Split from `agent_adapter_a.rs` for the 500-line ceiling. Spliced in with
// `include!` like its sibling, so it shares that file's imports and module
// scope; the caller is `validate_write_ownership`.

/// Grant this item the changed files nothing else in its wave claims.
///
/// The declared scope cannot be right in advance: a task's true write set is
/// discovered by reading the code, and a file that does not exist yet cannot be
/// claimed at all. Both directions failed live — one item declared too few
/// files and had a correct hour of work discarded for one unlisted path,
/// another declared 69 and collided.
///
/// So the scope is treated as a claim to be extended, not a prophecy to be
/// graded. Disjoint ownership is preserved exactly: a path no other item in the
/// wave owns cannot create a conflict by being granted here, and a contested
/// path is left to fail as before, which is a real ownership dispute and
/// belongs in remediation.
///
/// Returns the declared targets unchanged when there is no wave context, which
/// is every caller that has not been taught to supply one.
fn extend_scope_for_unclaimed_files(
    request: &WorkflowV2AgentRequest,
    result: &WorkflowV2Result,
    declared: Vec<String>,
    repository_root: Option<&str>,
) -> Vec<String> {
    let Some(wave) = wave_claims_of(&request.call) else {
        return declared;
    };
    let owned: std::collections::BTreeSet<&str> = declared.iter().map(String::as_str).collect();
    let outside: Vec<&str> = result
        .files_changed
        .iter()
        .map(|file| file.path.as_str())
        .filter(|path| !owned.contains(path))
        .collect();
    if outside.is_empty() {
        return declared;
    }
    // Claim paths and changed paths must be in the SAME form or an owned path
    // reads as unclaimed and gets granted — over-granting, silently, which is
    // worse than the discard this replaces. `normalize_items` already stores
    // plan targets repo-relative, so this is idempotent today; it is here so a
    // drift in that invariant cannot quietly disarm the contest check.
    //
    // A claim that will not normalise means the two forms cannot be compared at
    // all, so no extension is granted: fail closed, never open.
    let Some(wave) = normalized_claims(&request.call.id, wave, repository_root) else {
        return declared;
    };
    let (granted, contested) = crate::v2::write_scope_extension::resolve_scope_extensions(
        &request.call.id,
        outside,
        &wave,
    );
    // A contested path is deliberately NOT granted and NOT reported here: it
    // stays outside the declared scope, so `validate_changed_files` raises
    // ChangedFileOutsideOwnership naming that exact path, which is the message
    // the run already surfaces. This crate carries no logging dependency and
    // this is not the place to add one.
    let _ = &contested;
    if granted.is_empty() {
        return declared;
    }
    let mut extended = declared;
    extended.extend(granted);
    extended.sort();
    extended.dedup();
    extended
}

/// The wave's claim list, as the write coordinator stamped it onto the call.
///
/// `None` for every call that carries none — a read-only call, a serial write,
/// a test fixture — and ownership then stays strict, which is the pre-existing
/// behaviour. Deliberately not a bare `Vec`: an empty list reads as "no other
/// item claims anything", which would grant every out-of-scope write and
/// silently delete the ownership check. Malformed JSON is `None` for the same
/// reason. A plumbing mistake must fail closed, not open.
fn wave_claims_of(
    call: &WorkflowV2HostCall,
) -> Option<Vec<crate::v2::write_scope_extension::WaveClaim>> {
    serde_json::from_value(call.options.extra.get("wave_claims")?.clone()).ok()
}

/// Every claim rewritten into the same path form the changed files use.
///
/// `None` when any claim cannot be normalised: the comparison would then be
/// between two different path languages, and a mismatch grants rather than
/// refuses. Refusing to extend is the safe answer — it is exactly the
/// behaviour that exists today.
fn normalized_claims(
    item_id: &str,
    wave: Vec<crate::v2::write_scope_extension::WaveClaim>,
    repository_root: Option<&str>,
) -> Option<Vec<crate::v2::write_scope_extension::WaveClaim>> {
    wave.into_iter()
        .map(|claim| {
            let owned = claim
                .owned
                .iter()
                .map(|path| {
                    normalize_target_for_repository(item_id, path, repository_root).ok()
                })
                .collect::<Option<Vec<_>>>()?;
            Some(crate::v2::write_scope_extension::WaveClaim::new(
                claim.item_id,
                owned,
            ))
        })
        .collect()
}

