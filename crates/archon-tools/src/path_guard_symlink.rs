//! Symbolic links, for a write that has already been permitted by root.
//!
//! A root allowlist decides where a write may land by resolving the path and
//! comparing the result to a set of directories. That is sound at the instant
//! it runs and only then. A symbolic link is a name whose target is data: a
//! path that resolves inside the allowlist now can be made to resolve outside
//! it by rewriting one link, and nothing in the comparison notices, because the
//! comparison was correct about a path that no longer exists.
//!
//! Two things follow, and both are here rather than in the containment check
//! because they are a different question from "is this inside a root".
//!
//! **One check is not enough.** Canonicalising the target answers for the
//! target's own final component and for every directory above it *that already
//! exists*. It says nothing about a link created at the final component after
//! the check, and nothing about the lexical `..` removal that ran before it —
//! `normalize_lexically` deletes `a/link/../b` down to `a/b`, which is the
//! wrong file whenever `link` is a symlink to another directory. So the target
//! is checked, each component below the root is checked, and the resolved path
//! is checked, and they are checked for different reasons.
//!
//! **Refusing beats resolving.** A link inside a root that points to another
//! file inside the same root still passes containment, and a confined agent
//! that writes through it is writing to a path it never named. Under
//! confinement the honest answer is to refuse the link and make the agent name
//! the file it means. This is deliberately stricter than an unconfined session,
//! which is why it runs only when `write_roots` is non-empty: a user editing a
//! symlinked file in their own checkout is doing something ordinary, and a
//! workflow agent reaching a write target through one is not.

use std::path::{Component, Path};

/// Point one: the write target is itself a link.
///
/// Runs BEFORE the root containment check, and the order is load-bearing rather
/// than cosmetic. A link pointing out of the roots fails containment too, so
/// checking containment first would report it as "outside your directories" —
/// true, and the wrong diagnosis. The agent named a path that *is* inside its
/// directories; what it needs to be told is that the name is a link, because
/// that is the thing it can act on. Running first also means a link is refused
/// on the same grounds wherever it points, which is the actual rule.
///
/// `symlink_metadata` rather than `metadata`, since the whole difficulty is
/// that following the link is what hides it.
pub(crate) fn reject_symlinked_target(requested: &Path) -> Result<(), String> {
    if is_symlink(requested) {
        return Err(refusal(requested, requested));
    }
    Ok(())
}

/// Points two and three: the descent to the target, and the resolution of it.
///
/// `root` is the write root the resolved path was found under, `requested` the
/// lexically normalised path the caller asked for, and `resolved` what it
/// canonicalised to. All three are needed: the walk has to start somewhere
/// trusted (`root`), it has to follow the names the caller actually used
/// (`requested`) rather than the ones canonicalisation substituted, and the
/// final re-check has to be about the file that will really change
/// (`resolved`).
pub(crate) fn reject_symlinked_descent(
    root: &Path,
    requested: &Path,
    resolved: &Path,
) -> Result<(), String> {
    // 2. Every component between the root and the target. A link here diverts
    //    the whole subtree below it, and it is the component that lexical `..`
    //    removal silently mis-resolves.
    if let Some(offender) = symlinked_component_below(root, requested) {
        return Err(refusal(requested, &offender));
    }

    // 3. After resolution. Steps 1 and 2 walked the names; this asks whether
    //    the bytes that will change are still under the root that matched. It
    //    is not redundant with the caller's containment check — that one ran
    //    against the same `resolved`, and this is what keeps the guarantee true
    //    for a future caller that reorders them.
    if resolved != root && !resolved.starts_with(root) {
        return Err(format!(
            "Path '{}' resolves to '{}', which is outside the writable directory '{}'. \
             Name the file directly rather than reaching it through a link.",
            requested.display(),
            resolved.display(),
            root.display()
        ));
    }

    Ok(())
}

/// The first component strictly below `root` on the way to `requested` that is
/// a symbolic link.
///
/// The walk starts at the root and not at the filesystem root, because the
/// write roots are configuration the host chose: a link on the way *to* one is
/// the operator's own arrangement, and refusing those would refuse every write
/// on a machine where `/tmp` and `/var` are themselves links — which on macOS
/// is every machine.
///
/// Finding where the root sits inside `requested` is done by canonicalising
/// prefixes rather than by `strip_prefix`, and that is not fussiness. The roots
/// are canonical and the requested path is not, so on the same macOS the two
/// spellings never match as strings: `strip_prefix` returns nothing, the walk
/// is skipped, and the check silently stops existing on the platform it is
/// being written on. A guard that fails open on a spelling difference is the
/// defect, not a rounding error in it.
fn symlinked_component_below(root: &Path, requested: &Path) -> Option<std::path::PathBuf> {
    let mut walked = std::path::PathBuf::new();
    let mut components = requested.components().peekable();
    let mut inside_root = false;

    while let Some(component) = components.next() {
        match component {
            Component::Normal(part) => walked.push(part),
            Component::Prefix(prefix) => walked.push(prefix.as_os_str()),
            Component::RootDir => walked.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            // `normalize_lexically` has already removed these before the path
            // reaches here.
            Component::CurDir | Component::ParentDir => continue,
        }

        // The final component is step 1's job. Checking it here as well would
        // report one defect twice, with the less specific message.
        if components.peek().is_none() {
            break;
        }

        if !inside_root {
            // Not yet at the root, so this component is above it and not ours
            // to judge. It becomes ours the moment the prefix names the root.
            inside_root = std::fs::canonicalize(&walked).is_ok_and(|actual| actual == root);
            continue;
        }

        if is_symlink(&walked) {
            return Some(walked);
        }
    }
    None
}

/// Whether `path` is itself a symbolic link, without following it.
///
/// A path that does not exist is not a link — a write creating a new file is
/// the ordinary case and must not be refused for the absence of a link.
fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

fn refusal(requested: &Path, offender: &Path) -> String {
    format!(
        "Path '{}' reaches its target through the symbolic link '{}'. \
         This agent's writes are confined to declared directories, and a link is a \
         name whose destination can change after it is checked, so it is refused \
         rather than followed. Write to the real path instead.",
        requested.display(),
        offender.display()
    )
}

#[cfg(test)]
#[path = "path_guard_symlink_tests.rs"]
mod tests;
