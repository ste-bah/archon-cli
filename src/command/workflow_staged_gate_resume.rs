/// The staged set gate's budget and progress (Issue 259): the host's wall
/// clock for `task-set-lint`, counted from here, and a progress baseline on
/// `stderr` before anything slow (the critic client, the lint) runs. Without
/// it an attempt the host kills before its first saved batch reports no
/// progress at all, and the executor then reads the next attempt's saved
/// work as no evidence and pauses instead of retrying.
pub fn staged_set_gate_resume(
    stderr: &mut dyn std::io::Write,
) -> crate::command::workflow_freeze_budget::FreezeResume {
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("task-set-lint");
    let _ = writeln!(stderr, "{}", resume.progress.line());
    resume
}
