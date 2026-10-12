//! What the fidelity verdict store calls the critic's logic (#434c): the
//! files whose bytes decide what the critic is asked beyond the request
//! identity of the empty question, and how its reply becomes a verdict.
//!
//! `build.rs` mounts this file (`#[path]`) and hashes [`FILES`] with
//! [`digest`] into `ARCHON_FIDELITY_LOGIC_DIGEST`; the store keys every
//! verdict on it, so a binary rebuilt with these files unchanged reuses
//! every verdict, and any byte changed in one of them re-asks every
//! obligation. The binary's tests mount it too: the guard test fails when a
//! module of a listed file, or a fidelity file beside them, is in neither
//! [`FILES`] nor [`EXCLUDED`], so the list cannot silently fall behind the
//! code.

/// Hashed, sorted: the critic call, its prompt rendering, its reply reader
/// and checks (the `#[path]` modules of `fidelity_audit.rs`).
pub const FILES: &[&str] = &[
    "crates/archon-workflow/src/fidelity_audit.rs",
    "crates/archon-workflow/src/fidelity_audit_check.rs",
    "crates/archon-workflow/src/fidelity_audit_reply.rs",
    "src/command/topology_lint/fidelity_critic.rs",
];

/// Fidelity files beside [`FILES`] that are not hashed, each with the reason.
/// An entry here is a reviewed decision: hashing one of them would re-ask
/// every obligation on every edit to it, and each re-ask is a fresh chance
/// for the critic to flip a verdict on identical text (#434).
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
        "the one-time schema-4 migration; it is part of the store",
    ),
    (
        "src/command/topology_lint/fidelity_waivers.rs",
        "operator waivers applied to findings after the verdicts stand",
    ),
    (
        "src/command/topology_lint/fidelity_logic_digest.rs",
        "this list and its hash; a change to either changes the digest directly",
    ),
];

/// The digest of [`FILES`] as `read` returns them.
pub fn digest(read: impl FnMut(&str) -> Vec<u8>) -> String {
    digest_of(FILES, read)
}

/// SHA-256 over each of `files`' name and bytes, each framed by its length.
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
