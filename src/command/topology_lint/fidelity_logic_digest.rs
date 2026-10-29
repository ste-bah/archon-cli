//! What the fidelity verdict store calls the critic's logic (#434c, #434d):
//! the files whose bytes decide what the critic is asked beyond the request
//! identity of the empty question, and how its reply becomes a verdict.
//!
//! # A version, not a digest, in the key (#434d)
//!
//! The store keyed every verdict on a digest of [`FILES`], so any byte
//! changed in one of them — a re-ask wording, a comment, a refactor —
//! re-asked every obligation of a live run on the next deploy, and each
//! re-ask is a fresh chance for the critic to flip a verdict on identical
//! text (#434). The key now names [`CRITIC_LOGIC_VERSION`], which changes
//! only when the meaning of a stored verdict does: the question's wording,
//! what a verdict asserts, or an acceptance rule under which a verdict
//! accepted before would be refused now.
//!
//! A developer cannot change these files and forget the version: the
//! binary's tests pin the digest of [`FILES`] in [`SOURCES_DIGEST`] and fail
//! when they differ ([`pin_check`]). A binary built without that review —
//! only targeted tests run, or a dirty tree — is caught too: `build.rs`
//! mounts this file and hashes [`FILES`] into
//! `ARCHON_FIDELITY_SOURCES_DIGEST`, and a store whose build digest is not
//! [`SOURCES_DIGEST`] reads, migrates and writes no record. The fix is one of two: the change
//! cannot alter what a stored verdict means — set [`SOURCES_DIGEST`] to the
//! reported digest, with a comment saying why — or it can — bump
//! [`CRITIC_LOGIC_VERSION`] too, which re-asks every obligation once. The
//! guard tests also fail when a module of a listed file, or a fidelity file
//! beside them, is in neither [`FILES`] nor [`EXCLUDED`], so the list cannot
//! silently fall behind the code.

/// The meaning of a stored fidelity verdict. Bump only when a verdict
/// stored under the current version could mean something else, or be
/// refused, under the new code (see the module docs); a bump re-asks every
/// obligation once. Records written before this version existed — schema 4
/// and schema 5, keyed on the digest of [`FILES`] — are version 1 (see
/// `fidelity_store_legacy.rs`).
///
/// 1: the logic of schema 4 and schema 5 (#434, #434c).
/// 2: PRD acceptance/gap context, whole-set attribution and weakening evidence change the question.
/// #434d, same unreleased version 2: bounded quote normalization and ordered elision matching keep provenance checks literal.
/// #441, same unreleased version 2: elided support is expanded from one unique paragraph span and requires critic confirmation; all other evidence is exact or whole-quote normalized.
pub const CRITIC_LOGIC_VERSION: u32 = 2;

/// The digest of [`FILES`] as reviewed against [`CRITIC_LOGIC_VERSION`].
///
/// - `631274368c9492c557bedefacf3d203ce7072d51c4cdd682aff072608c741c7a`: the
///   files from bc8cf2693 (schema 4) to 22e024436 (schema 5), version 1.
/// - Re-pinned, version unchanged, for #434d: a reply is read verdict by
///   verdict and each verdict that passes every check a whole reply passes
///   is kept; only the rest are asked again, with a re-ask turn that names
///   them. No check is weakened and the question is unchanged, so a stored
///   verdict means what it meant.
/// - Re-pinned, version unchanged, for the #434d review: a kept verdict
///   answered again in the same conversation with the other meaning is
///   logged, naming both meanings, and nothing is recorded; the kept
///   verdict stays final. No acceptance rule changed.
/// - Re-pinned for #441 review, version 2: PRD source and policy context,
///   whole-set attribution, and weakening-evidence quotes change what a
///   verdict answers and when it blocks a body.
/// - #441 review, same unreleased version: the extracted whole-task-set
///   ownership lookup is included in the pinned logic files.
/// - #441 review 2, same unreleased version: Acceptance policy headings now
///   reach the critic question; vendor-contract acceptance headings remain excluded.
/// - #441 review 3, same unreleased version: the critic receives the complete PRD,
///   applies any policy anywhere in it, and sees each obligation's source heading.
/// - #434d, same unreleased version 2: quote provenance uses bounded text
///   normalization, elision expansion with critic confirmation, and rejection
///   of elided evidence in all other quote fields.
/// - #441 regression fix, version unchanged: exact supporting excerpts retain
///   their original quote for provenance matching, so a quote cut inside an
///   inline-code span is not rejected by re-normalizing unmatched Markdown.
/// - #441 regression fix, version unchanged: exact and whole-quote normalized
///   matches are checked before treating a task's literal ellipsis as elision.
/// - #441 follow-up, version unchanged: literal matches do not create elision
///   confirmation candidates before exact and normalized matching fails.
/// - #441 review, version 2 unchanged: quote normalization changes prose only;
///   Markdown code contents retain their bytes and cannot pass after alteration.
/// - D5, version 2 unchanged: an unusable or refused critic reply is typed as
///   the critic's own operational outcome; no verdict's meaning changes.
/// - C1, version 2 unchanged: a length stop before the first JSON token is
///   continued with the same reply, under the same checks; verdict meaning
///   does not change.
/// - G3 review, version 2 unchanged: a first cut reply that is not JSON is
///   asked again; how a whole reply is obtained changes, not what it means.
/// - Host availability recovery preserves provider fault types and diagnostics;
///   questions, reply checks and stored verdict meaning are unchanged.
// Typed idle recovery preserves the question, reply validation and stored verdict meaning.
// #497 W3 merge with main and review fixes, version unchanged: the gate evaluation travels with an incomplete lint and host faults keep their typed cause; no question, verdict assertion or acceptance rule changed.
pub const SOURCES_DIGEST: &str = "fc7bba195197c9813b1d63856c698fccaaab9fdfc76a99c47e01a1e8d1f13d05";

/// Hashed, sorted: the prompt, PRD source sections, reply checks, whole-set
/// fallback, and task-text-specific verdict reuse rules.
#[cfg_attr(not(test), allow(dead_code, reason = "read by the guard test"))]
pub const FILES: &[&str] = &[
    "crates/archon-workflow/src/fidelity_audit.rs",
    "crates/archon-workflow/src/fidelity_audit_check.rs",
    "crates/archon-workflow/src/fidelity_audit_context.rs",
    "crates/archon-workflow/src/fidelity_audit_normalize.rs",
    "crates/archon-workflow/src/fidelity_audit_reply.rs",
    "crates/archon-workflow/src/obligation_continuations.rs",
    "crates/archon-workflow/src/obligation_ids.rs",
    "crates/archon-workflow/src/obligation_source_sections.rs",
    "src/command/topology_lint/fidelity_candidate_fallback.rs",
    "src/command/topology_lint/fidelity_critic.rs",
    "src/command/topology_lint/fidelity_set_helpers.rs",
    "src/command/topology_lint/fidelity_store.rs",
    "src/command/workflow_task_set_judge_reply.rs",
];

/// Fidelity files beside [`FILES`] that are not hashed, each with the reason.
/// An entry here is a reviewed decision: none of them decides what the
/// critic is asked or when its reply is a verdict, so an edit to one needs
/// no review against [`CRITIC_LOGIC_VERSION`].
#[cfg_attr(not(test), allow(dead_code, reason = "read by the guard test"))]
pub const EXCLUDED: &[(&str, &str)] = &[
    (
        "src/command/topology_lint/fidelity.rs",
        "builds each question's data (obligation text, cluster tasks and their texts, \
         skeleton section) and reads standings after reuse; all of that data is in the \
         record key or bound per entry by value",
    ),
    (
        "src/command/topology_lint/fidelity_resume.rs",
        "chooses which obligations are asked and how they are batched; a record is per \
         obligation and deliberately does not name its batch partners",
    ),
    (
        "src/command/topology_lint/fidelity_store_legacy.rs",
        "the one-time migration of schema-4 and schema-5 records; it is part of the store",
    ),
    (
        "src/command/topology_lint/fidelity_waivers.rs",
        "operator waivers applied to findings after the verdicts stand",
    ),
    (
        "src/command/topology_lint/fidelity_logic_digest.rs",
        "this list, its hash, the pin and the version; a change to the list changes the \
         digest directly",
    ),
];

/// The digest of [`FILES`] as `read` returns them.
#[cfg_attr(not(test), allow(dead_code, reason = "read by the guard test"))]
pub fn digest(read: impl FnMut(&str) -> Vec<u8>) -> String {
    digest_of(FILES, read)
}

/// The guard: `Ok` when [`FILES`], as `read` returns them, hash to
/// `pinned`; otherwise what the developer must decide.
#[cfg_attr(not(test), allow(dead_code, reason = "read by the guard test"))]
pub fn pin_check(pinned: &str, read: impl FnMut(&str) -> Vec<u8>) -> Result<(), String> {
    let now = digest(read);
    if now == pinned {
        return Ok(());
    }
    Err(format!(
        "The fidelity critic's sources changed (#434d): their digest is {now}, pinned {pinned}.\n\
         In src/command/topology_lint/fidelity_logic_digest.rs: if the change can alter what a \
         stored verdict means (the question's wording, what a verdict asserts, or a rule under \
         which a verdict accepted before is refused now), bump CRITIC_LOGIC_VERSION; every \
         obligation is asked again once. If it cannot, keep the version. Either way set \
         SOURCES_DIGEST to the digest above, with a comment saying why."
    ))
}

/// SHA-256 over each of `files`' name and bytes, each framed by its length.
#[cfg_attr(not(test), allow(dead_code, reason = "read by the guard test"))]
pub fn digest_of(files: &[&str], mut read: impl FnMut(&str) -> Vec<u8>) -> String {
    use sha2::{Digest, Sha256};

    let mut hash = Sha256::new();
    for file in files {
        let contents = read(file);
        hash.update((file.len() as u64).to_le_bytes());
        hash.update(file.as_bytes());
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    format!("{:x}", hash.finalize())
}
