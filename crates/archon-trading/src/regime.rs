use serde::{Deserialize, Serialize};

pub const REALIZED_VOL_WINDOW: usize = 20;
pub const REGIME_BOUNDARY_CHANGE_BPS: i64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegimeLabel {
    pub session_index: usize,
    pub regime_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VolatilityPoint {
    pub session_index: usize,
    pub realized_vol: Option<f64>,
    pub regime_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegimeError {
    NonFiniteReturn { index: usize },
    NonPositiveClose { index: usize },
}

pub fn classify_return_regimes(returns: &[f64]) -> Result<Vec<RegimeLabel>, RegimeError> {
    Ok(classify_return_points(returns)?
        .into_iter()
        .map(|point| RegimeLabel {
            session_index: point.session_index,
            regime_id: point.regime_id,
        })
        .collect())
}

pub fn classify_return_points(returns: &[f64]) -> Result<Vec<VolatilityPoint>, RegimeError> {
    validate_returns(returns)?;
    let mut regime_id = 0;
    let mut previous_complete_vol = None;
    let mut points = Vec::with_capacity(returns.len());

    for session_index in 0..returns.len() {
        let realized_vol = realized_vol_at(returns, session_index, REALIZED_VOL_WINDOW);
        if is_boundary(previous_complete_vol, realized_vol) {
            regime_id += 1;
        }
        if is_boundary(previous_complete_vol, realized_vol) {
            previous_complete_vol = realized_vol;
        } else if previous_complete_vol.is_none() && realized_vol.is_some() {
            previous_complete_vol = realized_vol;
        }
        points.push(VolatilityPoint {
            session_index,
            realized_vol,
            regime_id,
        });
    }

    Ok(points)
}

pub fn classify_close_regimes(closes: &[f64]) -> Result<Vec<RegimeLabel>, RegimeError> {
    let returns = close_to_returns(closes)?;
    let return_labels = classify_return_regimes(&returns)?;
    let mut labels = Vec::with_capacity(closes.len());
    if !closes.is_empty() {
        labels.push(RegimeLabel {
            session_index: 0,
            regime_id: 0,
        });
    }
    labels.extend(return_labels.into_iter().map(|label| RegimeLabel {
        session_index: label.session_index + 1,
        regime_id: label.regime_id,
    }));
    Ok(labels)
}

fn validate_returns(returns: &[f64]) -> Result<(), RegimeError> {
    for (index, value) in returns.iter().enumerate() {
        if !value.is_finite() {
            return Err(RegimeError::NonFiniteReturn { index });
        }
    }
    Ok(())
}

fn close_to_returns(closes: &[f64]) -> Result<Vec<f64>, RegimeError> {
    for (index, close) in closes.iter().enumerate() {
        if !close.is_finite() || *close <= 0.0 {
            return Err(RegimeError::NonPositiveClose { index });
        }
    }
    Ok(closes
        .windows(2)
        .map(|pair| pair[1] / pair[0] - 1.0)
        .collect())
}

fn realized_vol_at(returns: &[f64], session_index: usize, window: usize) -> Option<f64> {
    if window == 0 || session_index + 1 < window {
        return None;
    }
    let start = session_index + 1 - window;
    let sum_squares: f64 = returns[start..=session_index]
        .iter()
        .map(|value| value * value)
        .sum();
    Some((sum_squares / window as f64).sqrt())
}

fn is_boundary(previous: Option<f64>, current: Option<f64>) -> bool {
    match (previous, current) {
        (Some(previous), Some(current)) if previous == 0.0 => current > 0.0,
        (Some(previous), Some(current)) => boundary_ratio(previous, current) >= 0.5,
        _ => false,
    }
}

#[cfg(test)]
fn boundary_bps(previous: f64, current: f64) -> i64 {
    (boundary_ratio(previous, current) * 10_000.0).floor() as i64
}

fn boundary_ratio(previous: f64, current: f64) -> f64 {
    (current - previous).abs() / previous
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_data_04_is_deterministic_and_marks_fifty_percent_vol_boundary() {
        let mut returns = vec![0.01; 20];
        returns.extend([0.02; 20]);

        let first = classify_return_points(&returns).expect("valid returns");
        let second = classify_return_points(&returns).expect("valid returns");

        assert_eq!(first, second);
        assert_eq!(first[18].realized_vol, None);
        assert_eq!(first[19].regime_id, 0);
        assert_eq!(first[19].realized_vol, Some(0.01));
        assert_eq!(first[39].regime_id, 1);
        assert!(first[39].realized_vol.expect("windowed vol") >= 0.02);
    }

    #[test]
    fn boundary_is_not_triggered_below_fifty_percent() {
        let mut returns = vec![0.01; 20];
        returns.extend([0.0149; 20]);

        let labels = classify_return_regimes(&returns).expect("valid returns");

        assert!(labels.iter().all(|label| label.regime_id == 0));
    }

    #[test]
    fn boundary_is_not_rounded_up_at_just_below_fifty_percent() {
        let mut returns = vec![0.01; 20];
        returns.extend([0.0149995; 20]);

        let labels = classify_return_regimes(&returns).expect("valid returns");

        assert!(labels.iter().all(|label| label.regime_id == 0));
        assert_eq!(boundary_bps(0.01, 0.0149995), 4_999);
    }

    #[test]
    fn close_series_maps_labels_to_original_session_indexes() {
        let closes: Vec<f64> = (0..=21).map(|index| 100.0 + index as f64).collect();

        let labels = classify_close_regimes(&closes).expect("valid closes");

        assert_eq!(labels.len(), closes.len());
        assert_eq!(labels[0].session_index, 0);
        assert_eq!(labels[21].session_index, 21);
    }

    #[test]
    fn rejects_non_finite_returns_without_io_or_rng() {
        let error = classify_return_regimes(&[0.01, f64::NAN]).expect_err("nan rejected");

        assert_eq!(error, RegimeError::NonFiniteReturn { index: 1 });
    }
}
