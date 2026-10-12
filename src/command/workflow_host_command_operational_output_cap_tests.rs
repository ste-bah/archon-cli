use super::*;

#[test]
fn output_byte_count_does_not_make_a_successful_exit_operational() {
    let mut output = output(Some(0), false, "bounded head and tail");
    output.stdout_bytes = 6 * 1024 * 1024;
    output.stderr_bytes = 6 * 1024 * 1024;
    assert_eq!(classify(&output), None);
}

#[test]
fn round3_growing_operational_progress_has_no_total_attempt_limit() {
    let mut history: Vec<_> = (1..=130).map(|n| attempt(n, Some(u64::from(n)))).collect();
    assert_eq!(next_step(&history), NextStep::Retry);
    history.push(attempt(131, Some(130)));
    assert_eq!(next_step(&history), NextStep::Pause("no_progress"));
}
