// The stage runs its checks through the POSIX process-group runner; the
// tests execute `test -f` criteria for real, so they are Unix-only like it.
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_author_stage_tests.rs"]
mod author_stage_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_crash_tests.rs"]
mod crash_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_refused_tests.rs"]
mod refused_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_repair_tests.rs"]
mod repair_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_stalled_tests.rs"]
mod stalled_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_replay_tests.rs"]
mod replay_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_tests.rs"]
mod tests;
