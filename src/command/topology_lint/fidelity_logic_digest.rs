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
pub const CRITIC_LOGIC_VERSION: u32 = 1;

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
pub const SOURCES_DIGEST: &str = "8558044b0a09b9665085f65be84237450894456539afbc40b6ef34ad23d6bee4";

/// Hashed, sorted: the critic call, its prompt rendering, its reply reader
/// and checks (the `#[path]` modules of `fidelity_audit.rs`).
#[cfg_attr(not(test), allow(dead_code, reason = "read by the guard test"))]
pub const FILES: &[&str] = &[
    "crates/archon-workflow/src/fidelity_audit.rs",
    "crates/archon-workflow/src/fidelity_audit_check.rs",
    "crates/archon-workflow/src/fidelity_audit_reply.rs",
    "src/command/topology_lint/fidelity_critic.rs",
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
        "src/command/topology_lint/fidelity_store.rs",
        "the reuse rules themselves; a change to the record layout bumps STORE_SCHEMA",
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
