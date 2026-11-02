//! What the closure walk stops at: the denylist rule, and the files under a
//! denied directory that it hashes anyway (Issue 288).
use std::path::Path;

use super::Workspace;

/// Files under a `LOGIC_DENYLIST` directory that are hashed anyway, each
/// with the reason. Issue 288: a bounded acceptance check verdict is reused
/// across builds by logic version and digest, so the code that shapes the
/// check's process (environment, argv, cwd, shell) must move the digest. The
/// guard test fails on an exception no capability hashes.
pub(crate) const LOGIC_DENYLIST_EXCEPTIONS: &[(&str, &str)] = &[
    (
        "crates/archon-tools/src/workflow_resource_env.rs",
        "sets the resource environment of every acceptance check process just before launch",
    ),
    (
        "crates/archon-tools/src/build_cache_env.rs",
        "decides the build cache environment and toolchain markers of an acceptance check process",
    ),
    (
        "crates/archon-tools/src/bash_env.rs",
        "fills environment defaults the resource environment sets for an acceptance check process",
    ),
];

impl Workspace {
    pub(super) fn denied<'d>(rel: &Path, deny: &[(&'d str, &str)]) -> Option<&'d str> {
        let excepted = LOGIC_DENYLIST_EXCEPTIONS
            .iter()
            .any(|(file, _)| rel == Path::new(file));
        if excepted {
            return None;
        }
        deny.iter()
            .map(|(entry, _)| *entry)
            .find(|entry| rel == Path::new(entry) || entry.ends_with('/') && rel.starts_with(entry))
    }
}
