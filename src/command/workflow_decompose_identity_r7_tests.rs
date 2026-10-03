//! The live R7 decomposition resumes only while the embedded script and the
//! host command catalog it launched with are unchanged (`verify_fixed_resume_identity`):
//! a binary hot-patched into it must reproduce both digests exactly.

/// R7's persisted resume identity (`decomposition/state.json` of run
/// wf-913e62ae): the catalog is rebuilt under the launch revision.
const R7_LAUNCH_REVISION: &str = "f483aed53";
const R7_SCRIPT_DIGEST: &str = "f61a7d7a806327894f9bfb73fe191d0872c6b0719fb7f2cb19cd2f1aafda43d9";
const R7_CATALOG_DIGEST: &str = "a27865489b144043b49ab8d5692d46e1d3d6fa7487dbe99efebd7e74cf155b07";

#[test]
fn this_binary_keeps_the_r7_script_and_catalog_digests() {
    assert_eq!(
        archon_workflow::workflow_scaffold_hash(
            crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE
        ),
        R7_SCRIPT_DIGEST
    );
    let catalog = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog(
        R7_LAUNCH_REVISION,
    )
    .unwrap();
    assert_eq!(catalog.digest, R7_CATALOG_DIGEST);
}
