use archon_workflow::v2::{
    WorkflowV2HarnessError, WorkflowV2HarnessValidator, WorkflowV2HostMethod, WorkflowV2WriteMode,
};

fn validator() -> WorkflowV2HarnessValidator {
    WorkflowV2HarnessValidator
}

include!("v2_harness_validation_parts/a.rs");
include!("v2_harness_validation_parts/b.rs");
include!("v2_harness_validation_parts/c.rs");
include!("v2_harness_validation_parts/d.rs");
