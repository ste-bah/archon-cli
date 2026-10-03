use super::super::{
    WorkflowV2CallExecution, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind, WorkflowV2WriteMode,
};
use super::*;

include!("result_store_tests_a.rs");
include!("result_store_tests_b.rs");
include!("result_store_tests_scan.rs");
include!("result_store_tests_history.rs");
include!("result_store_tests_history_scope.rs");
include!("result_store_tests_archive.rs");
