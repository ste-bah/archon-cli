//! A host-launched agent key must resolve in every project.
//!
//! `workflow freeze-acceptance --reauthor` and the in-round acceptance repair
//! launch `ACCEPTANCE_REAUTHOR_AGENT`; before it was registered, every project
//! without a same-named `.archon/agents` file failed the launch with
//! "Unknown subagent type". The fixture's project has no agent directories.

use super::is_write_capable;
use super::spawn_cache_tests::fixture;
use crate::agents::harness::{ACCEPTANCE_REAUTHOR_AGENT, HOST_READ_ONLY_TOOLS};

#[test]
fn the_acceptance_reauthor_resolves_through_the_executor_in_a_bare_project() {
    let (executor, _project) = fixture(None);
    let def = executor
        .resolve_agent(ACCEPTANCE_REAUTHOR_AGENT)
        .expect("the host's re-author key resolves with no .archon/agents");
    assert_eq!(
        def.allowed_tools.as_deref(),
        Some(&HOST_READ_ONLY_TOOLS.map(String::from)[..])
    );
    assert!(
        !is_write_capable(Some(&def)),
        "the re-author definition is read-only"
    );
}

#[test]
fn an_unregistered_key_does_not_resolve_through_the_executor() {
    let (executor, _project) = fixture(None);
    assert!(
        executor
            .resolve_agent("acceptance-reauthor-unregistered")
            .is_none()
    );
}
