use super::ArchonConfig;

/// Describe configured write budgets that cannot cover one full host call and
/// leave a full host-call timeout available for retries and size re-asks.
pub fn write_call_time_budget_warnings(config: &ArchonConfig) -> Vec<String> {
    let generated = &config.workflow.generated;
    let host_call_timeout_secs = generated.host_call_timeout_secs;
    let write_call_time_budget_secs = generated.write_call_time_budget_secs;

    if write_call_time_budget_secs == 0
        || u64::from(write_call_time_budget_secs) >= 2 * u64::from(host_call_timeout_secs)
    {
        return Vec::new();
    }

    let minimum_secs = 2 * u64::from(host_call_timeout_secs);
    let default_secs = 3 * u64::from(host_call_timeout_secs);
    vec![format!(
        "workflow.generated.write_call_time_budget_secs = {write_call_time_budget_secs}s is below 2 × workflow.generated.host_call_timeout_secs = {host_call_timeout_secs}s ({minimum_secs}s); after one full host call timeout, less than one host-call timeout remains for size re-asks and transport retries. The default is 3 × workflow.generated.host_call_timeout_secs ({default_secs}s)."
    )]
}
