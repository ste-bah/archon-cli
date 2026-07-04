#[cfg(test)]
pub(super) fn ahdm_position_size(account_equity: f64, entry: f64, stop: f64) -> Option<f64> {
    let risk_per_unit = (entry - stop).abs();
    if !account_equity.is_finite()
        || !entry.is_finite()
        || !stop.is_finite()
        || account_equity <= 0.0
        || entry <= 0.0
        || risk_per_unit <= 0.0
    {
        return None;
    }
    Some(((account_equity * 0.005) / risk_per_unit).min((account_equity * 0.01) / entry))
}
