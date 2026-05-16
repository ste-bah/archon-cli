//! `archon world guard` command rendering and policy helpers.

include!("guard/00_runtime.rs");
include!("guard/01_pipeline.rs");
include!("guard/02_classify_status.rs");
include!("guard/03_commands.rs");

#[cfg(test)]
mod tests {
    include!("guard/04_tests_policy.rs");
    include!("guard/05_tests_pipeline.rs");
    include!("guard/06_tests_status.rs");
    include!("guard/07_tests_manual_events.rs");
}
