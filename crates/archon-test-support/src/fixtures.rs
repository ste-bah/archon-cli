//! The workflow-live JSON fixture corpus, `include_str!`d once.
//!
//! # Why the corpus lives here and not beside its tests
//!
//! `include_str!` resolves relative to the *source file*, so a fixture can only
//! be read by tests that sit at a fixed relative offset from it. Phase 2 moves
//! `src/command/workflow*.rs` into `archon-workflow` in clusters, and the
//! clusters share fixtures: a fixture read by a cluster that moves and by one
//! that stays cannot follow either of them. The two obvious ways out were both
//! rejected — a cross-crate `include_str!("../../src/command/fixtures/...")`
//! reaches through the package boundary phase 2 exists to establish (and breaks
//! `crate2nix`), and duplicating the file into both crates invites the two
//! copies to drift apart silently.
//!
//! So the corpus has one home, at a fixed offset from this file, and every
//! consumer names a `const` instead of a path. A test in the binary and a test
//! in `archon-workflow` then read the same bytes with no relative path between
//! them, and a cluster can move between crates without touching its fixtures.
//!
//! # Why all of it, not only the shared subset
//!
//! Which fixtures are "shared" changes every time a cluster moves, so a split
//! corpus would have to be re-derived each wave — reintroducing exactly the
//! drift this module removes. These are `pub` in a library crate, so an
//! entry no test currently reads is not dead code and costs nothing to keep.
//!
//! Consumers take `archon-test-support` as a `[dev-dependencies]` entry; the
//! crate never enters a production dependency graph.

pub const D26_COMPOUND_REPAIR_FAILURE: &str =
    include_str!("../fixtures/d26_compound_repair_failure.json");
pub const D31_REPEATED_GAP_RETRY_CHAIN: &str =
    include_str!("../fixtures/d31_repeated_gap_retry_chain.json");
pub const D32_ZERO_MATCH_RETRIAGE: &str = include_str!("../fixtures/d32_zero_match_retriage.json");
pub const D33_VERIFICATION_REMEDIATION_CONTRACT_FAILURE: &str =
    include_str!("../fixtures/d33_verification_remediation_contract_failure.json");
pub const D36_RAW_ARTIFACT_OVERREACH_NOOP_LOOP: &str =
    include_str!("../fixtures/d36_raw_artifact_overreach_noop_loop.json");
pub const D43_SAME_TASK_VERIFICATION_REMEDIATION: &str =
    include_str!("../fixtures/d43_same_task_verification_remediation.json");
pub const D46_ORPHANED_VERIFICATION_RETRY: &str =
    include_str!("../fixtures/d46_orphaned_verification_retry.json");
pub const D60_REFUTED_NOOP_ROUTING: &str =
    include_str!("../fixtures/d60_refuted_noop_routing.json");
pub const D61_NOOP_ACCEPTANCE_CONSISTENCY: &str =
    include_str!("../fixtures/d61_noop_acceptance_consistency.json");
pub const D63_RETRIAGE_RETRY_CONSUMER: &str =
    include_str!("../fixtures/d63_retriage_retry_consumer.json");
pub const D71_POST_REMEDIATION_TRIAGE_ENVELOPE: &str =
    include_str!("../fixtures/d71_post_remediation_triage_envelope.json");
pub const REVIEW_REMEDIATION_ARTIFACT_ONLY_ITEM: &str =
    include_str!("../fixtures/review_remediation_artifact_only_item.json");
pub const WF0ECA_VERIFICATION_REPAIR_PLAN_1_1_ITEM: &str =
    include_str!("../fixtures/wf0eca_verification_repair_plan_1_1_item.json");
pub const WF0ECA_VERIFICATION_REPAIR_PLAN_1_2_ITEM: &str =
    include_str!("../fixtures/wf0eca_verification_repair_plan_1_2_item.json");
pub const WF139_PROJECT_ARTIFACT_WRITE_FALSE_SAFETY: &str =
    include_str!("../fixtures/wf139_project_artifact_write_false_safety.json");
pub const WF139E_BLOCKED_VERIFICATION_FAILED_SOURCE_RESULT: &str =
    include_str!("../fixtures/wf139e_blocked_verification_failed_source_result.json");
pub const WF139E_VERIFICATION_PLAN_ITEMS: &str =
    include_str!("../fixtures/wf139e_verification_plan_items.json");
pub const WF139E_VERIFICATION_REPAIR_PLAN_1_1_ITEMS: &str =
    include_str!("../fixtures/wf139e_verification_repair_plan_1_1_items.json");
pub const WF139E_VERIFICATION_REPAIR_PLAN_1_2_ITEMS: &str =
    include_str!("../fixtures/wf139e_verification_repair_plan_1_2_items.json");
pub const WF139E_VERIFICATION_REPAIR_PLAN_1_3_ITEMS: &str =
    include_str!("../fixtures/wf139e_verification_repair_plan_1_3_items.json");
pub const WF199_VERIFICATION_REPAIR_PLAN_1_1: &str =
    include_str!("../fixtures/wf199_verification_repair_plan_1_1.json");
pub const WF19F5_BLOCKED_VERIFICATION_FAILED_1: &str =
    include_str!("../fixtures/wf19f5_blocked_verification_failed_1.json");
pub const WF19F5_VERIFICATION_REPAIR_PLAN_1_1: &str =
    include_str!("../fixtures/wf19f5_verification_repair_plan_1_1.json");
pub const WF19F5_VERIFICATION_REPAIR_PLAN_1_3: &str =
    include_str!("../fixtures/wf19f5_verification_repair_plan_1_3.json");
pub const WF19F5_VERIFICATION_WAVE_1_3_REJECTED: &str =
    include_str!("../fixtures/wf19f5_verification_wave_1_3_rejected.json");
pub const WF1CA_VERIFICATION_REPAIR_PLAN_1_1: &str =
    include_str!("../fixtures/wf1ca_verification_repair_plan_1_1.json");
pub const WF28F_REVIEW_REMEDIATION_DUPLICATE_TASK_ITEMS: &str =
    include_str!("../fixtures/wf28f_review_remediation_duplicate_task_items.json");
pub const WF2D24_BLOCKED_VERIFICATION_FAILED_1: &str =
    include_str!("../fixtures/wf2d24_blocked_verification_failed_1.json");
pub const WF2D24_IMPLEMENTATION_WAVE_1_NOOP: &str =
    include_str!("../fixtures/wf2d24_implementation_wave_1_noop.json");
pub const WF2D24_VERIFICATION_WAVE_1_3_DATA_STORE_FAILED: &str =
    include_str!("../fixtures/wf2d24_verification_wave_1_3_data_store_failed.json");
pub const WF32_ARTIFACT_VERIFICATION_AGGREGATE: &str =
    include_str!("../fixtures/wf32_artifact_verification_aggregate.json");
pub const WF32_VERIFICATION_INVARIANT_CHAIN: &str =
    include_str!("../fixtures/wf32_verification_invariant_chain.json");
pub const WF346_VERIFICATION_MISSING_PROJECT_ARTIFACTS: &str =
    include_str!("../fixtures/wf346_verification_missing_project_artifacts.json");
pub const WF346_VERIFICATION_PROJECT_RELATIVE_ITEM: &str =
    include_str!("../fixtures/wf346_verification_project_relative_item.json");
pub const WF3B9_VERIFICATION_FAILURE_TRIAGE_5_3: &str =
    include_str!("../fixtures/wf3b9_verification_failure_triage_5_3.json");
pub const WF44_HOST_MANIFEST_SCHEMA_OVERREACH: &str =
    include_str!("../fixtures/wf44_host_manifest_schema_overreach.json");
pub const WF485_VERIFICATION_REMEDIATION_SOURCE_ITEM: &str =
    include_str!("../fixtures/wf485_verification_remediation_source_item.json");
pub const WF485_VERIFICATION_RETRY_MERGE: &str =
    include_str!("../fixtures/wf485_verification_retry_merge.json");
pub const WF580_REVIEW_REMEDIATION_INVENTORY_ITEMS: &str =
    include_str!("../fixtures/wf580_review_remediation_inventory_items.json");
pub const WF580_REVIEW_VERIFICATION_PLAN_ITEMS: &str =
    include_str!("../fixtures/wf580_review_verification_plan_items.json");
pub const WF66_REMEDIATION_WAVE_1_3_SOURCE_PREFLIGHT: &str =
    include_str!("../fixtures/wf66_remediation_wave_1_3_source_preflight.json");
pub const WF66_REVIEW_REMEDIATION_WAVE_FAILED_MINIMAL: &str =
    include_str!("../fixtures/wf66_review_remediation_wave_failed_minimal.json");
pub const WF6C30_DEADLOCK_INVENTORY: &str =
    include_str!("../fixtures/wf6c30_deadlock_inventory.json");
pub const WF6CC_DEPENDENCY_GRAPH_REPAIR_2: &str =
    include_str!("../fixtures/wf6cc_dependency_graph_repair_2.json");
pub const WF6DD_VERIFICATION_RETRY_INVARIANT_FAILURE: &str =
    include_str!("../fixtures/wf6dd_verification_retry_invariant_failure.json");
pub const WF835_FOLLOWUP_REMEDIATION_SIDE_TASK: &str =
    include_str!("../fixtures/wf835_followup_remediation_side_task.json");
pub const WF90070_CANONICAL_INVENTORY: &str =
    include_str!("../fixtures/wf90070_canonical_inventory.json");
pub const WF98_IMPLEMENTATION_WAVE_1_MODULE_CHILD_OWNERSHIP: &str =
    include_str!("../fixtures/wf98_implementation_wave_1_module_child_ownership.json");
pub const WF98_IMPLEMENTATION_WAVE_2_FALSE_SAFETY: &str =
    include_str!("../fixtures/wf98_implementation_wave_2_false_safety.json");
pub const WFAB880_OWNERSHIP_EXPANSION_NEEDED: &str =
    include_str!("../fixtures/wfab880_ownership_expansion_needed.json");
pub const WFB36_OWNED_DIFF_SCOPE_FAILURE: &str =
    include_str!("../fixtures/wfb36_owned_diff_scope_failure.json");
pub const WFB36_OWNED_DIFF_SCOPE_RETRY_FAILURE: &str =
    include_str!("../fixtures/wfb36_owned_diff_scope_retry_failure.json");
pub const WFC022_EMPTY_PATCH_NO_NOOP_BRANCH_FAILURE: &str =
    include_str!("../fixtures/wfc022_empty_patch_no_noop_branch_failure.json");
pub const WFC5D4_VERIFICATION_REPAIR_PLAN_1_3: &str =
    include_str!("../fixtures/wfc5d4_verification_repair_plan_1_3.json");
pub const WFCAC_VERIFICATION_REPAIR_CONSOLIDATED_RETRY_ITEM: &str =
    include_str!("../fixtures/wfcac_verification_repair_consolidated_retry_item.json");
pub const WFCD824_BLOCKED_VERIFICATION_FAILED_RESULT: &str =
    include_str!("../fixtures/wfcd824_blocked_verification_failed_result.json");
pub const WFCD824_VERIFICATION_WAVE_1_3_CHECK_1: &str =
    include_str!("../fixtures/wfcd824_verification_wave_1_3_check_1.json");
pub const WFCD824_VERIFICATION_WAVE_1_3_CHECK_2: &str =
    include_str!("../fixtures/wfcd824_verification_wave_1_3_check_2.json");
pub const WFD009_REMEDIATION_INVENTORY_INVALID_TASK: &str =
    include_str!("../fixtures/wfd009_remediation_inventory_invalid_task.json");
pub const WFDC_SIZE_POLICY_BRANCH_FAILURE: &str =
    include_str!("../fixtures/wfdc_size_policy_branch_failure.json");
pub const WFE2C_MISSING_PROJECT_ARTIFACT_FALSE_SAFETY: &str =
    include_str!("../fixtures/wfe2c_missing_project_artifact_false_safety.json");
pub const WFF68_VERIFICATION_REPAIR_PLAN_1_1: &str =
    include_str!("../fixtures/wff68_verification_repair_plan_1_1.json");
pub const WFF9_PROVIDER_MISSING_ENV_PROOF_RESULT: &str =
    include_str!("../fixtures/wff9_provider_missing_env_proof_result.json");
pub const WFFE12_BLOCKED_VERIFICATION_FAILED_1: &str =
    include_str!("../fixtures/wffe12_blocked_verification_failed_1.json");
pub const WFFE12_VERIFICATION_REPAIR_PLAN_1_3_ITEMS: &str =
    include_str!("../fixtures/wffe12_verification_repair_plan_1_3_items.json");
pub const WFFE12_VERIFICATION_WAVE_1_3_SOURCE_REJECTED: &str =
    include_str!("../fixtures/wffe12_verification_wave_1_3_source_rejected.json");
pub const WFFE96_ARTIFACT_REQUIREMENTS_DISCOVERY_3_ITEMS: &str =
    include_str!("../fixtures/wffe96_artifact_requirements_discovery_3_items.json");
pub const WFFED_BLOCKED_VERIFICATION_FAILED_1: &str =
    include_str!("../fixtures/wffed_blocked_verification_failed_1.json");
pub const WFFED_VERIFICATION_FAILURE_TRIAGE_1_2: &str =
    include_str!("../fixtures/wffed_verification_failure_triage_1_2.json");
pub const WFFED_VERIFICATION_PLAN_1: &str =
    include_str!("../fixtures/wffed_verification_plan_1.json");
pub const WFFED_VERIFICATION_REMEDIATION_INVENTORY_1_1_NOOP: &str =
    include_str!("../fixtures/wffed_verification_remediation_inventory_1_1_noop.json");
pub const WFFED_VERIFICATION_REPAIR_SHAPE_REPAIR_1_1_1: &str =
    include_str!("../fixtures/wffed_verification_repair_shape_repair_1_1_1.json");
pub const WFFED_VERIFICATION_WAVE_1_1_BAD_BRANCH: &str =
    include_str!("../fixtures/wffed_verification_wave_1_1_bad_branch.json");
pub const WRITE_VALIDATION_ERROR_SOURCE_IDENTITY: &str =
    include_str!("../fixtures/write_validation_error_source_identity.json");
