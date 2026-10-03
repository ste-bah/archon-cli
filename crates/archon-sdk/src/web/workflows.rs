include!("workflows_parts/part_1.rs");
include!("workflows_parts/part_2.rs");

#[cfg(test)]
#[path = "workflow_history_tests.rs"]
mod history_tests;
