//! ACC-A7 / M4: an extension's gate holds the coverage a whole-set freeze
//! holds -- every requirement of the PRD as it is now covered by a check.

use super::*;
use crate::command::workflow_task_set::republish::test_fixture::{
    NO_SEEDS, criterion, frozen_set_in,
};

#[test]
fn an_extension_that_leaves_a_requirement_uncovered_is_refused_by_its_gate() {
    let set = frozen_set_in(
        &[("AC-F-001", "test -f present", true)],
        FreezeGateMode::Enforce,
        "- REQ-F-001: the store keeps raw responses.\n- REQ-F-002: the store is append-only.\n",
    );
    let before = set.chain_bytes();
    let frozen = set.contract();
    let mut sup = criterion("SUP-REQ-F-001", "test -f present", true);
    sup.covers = vec!["REQ-F-001".into()];
    let extension = Extension {
        entries: vec![sup],
        prd_digest: frozen.prd.digest.clone(),
        frozen_prd_digest: frozen.prd.digest.clone(),
        // Claimed together, so only the coverage can refuse it.
        new_obligations: ["REQ-F-001".to_string(), "REQ-F-002".to_string()].into(),
    };
    let ids: BTreeSet<String> = ["SUP-REQ-F-001".to_string()].into();
    let error = extend_and_republish(
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &ids,
            gate: ReauthorGate {
                probe: &set.probe,
                seeds: &NO_SEEDS,
            },
            trigger: "test extension",
        },
        &extension,
    )
    .expect_err("REQ-F-002 is still covered by no check");
    assert!(format!("{error:#}").contains("REQ-F-002"), "{error:#}");
    assert_eq!(set.chain_bytes(), before, "nothing published");
}
