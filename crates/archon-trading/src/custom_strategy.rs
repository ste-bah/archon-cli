use crate::ohlcv::OhlcvBar;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomOhlcvStrategy {
    pub name: Option<String>,
    pub entry: Vec<OhlcvCondition>,
    pub exit: Vec<OhlcvCondition>,
    #[serde(default)]
    pub min_hold_bars: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhlcvCondition {
    pub left: OhlcvOperand,
    pub op: ComparisonOp,
    pub right: OhlcvOperand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OhlcvOperand {
    Field {
        field: OhlcvField,
    },
    Indicator {
        indicator: OhlcvIndicator,
        len: Option<usize>,
    },
    Constant {
        value: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OhlcvField {
    Open,
    High,
    Low,
    Close,
    Volume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OhlcvIndicator {
    Sma,
    PrevClose,
    ChangePct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOp {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomStrategyError {
    MissingConditions(&'static str),
    InvalidIndicatorLength,
}

pub fn validate_custom_strategy(strategy: &CustomOhlcvStrategy) -> Result<(), CustomStrategyError> {
    if strategy.entry.is_empty() {
        return Err(CustomStrategyError::MissingConditions("entry"));
    }
    if strategy.exit.is_empty() {
        return Err(CustomStrategyError::MissingConditions("exit"));
    }
    for condition in strategy.entry.iter().chain(strategy.exit.iter()) {
        validate_operand(&condition.left)?;
        validate_operand(&condition.right)?;
    }
    Ok(())
}

pub fn custom_entry_exit_pairs(
    strategy: &CustomOhlcvStrategy,
    bars: &[OhlcvBar],
) -> Result<Vec<(usize, usize)>, CustomStrategyError> {
    validate_custom_strategy(strategy)?;
    let mut pairs = Vec::new();
    let mut entry = None;
    for index in 1..bars.len() {
        if entry.is_none() && all_conditions(&strategy.entry, bars, index) {
            entry = Some(index);
        } else if let Some(entry_index) = entry
            && index >= entry_index + strategy.min_hold_bars
            && all_conditions(&strategy.exit, bars, index)
        {
            pairs.push((entry_index, index));
            entry = None;
        }
    }
    if let Some(entry_index) = entry {
        pairs.push((entry_index, bars.len().saturating_sub(1)));
    }
    Ok(pairs)
}

fn all_conditions(conditions: &[OhlcvCondition], bars: &[OhlcvBar], index: usize) -> bool {
    conditions
        .iter()
        .all(|condition| condition_matches(condition, bars, index))
}

fn condition_matches(condition: &OhlcvCondition, bars: &[OhlcvBar], index: usize) -> bool {
    let Some(left) = operand_value(&condition.left, bars, index) else {
        return false;
    };
    let Some(right) = operand_value(&condition.right, bars, index) else {
        return false;
    };
    match condition.op {
        ComparisonOp::Gt => left > right,
        ComparisonOp::Gte => left >= right,
        ComparisonOp::Lt => left < right,
        ComparisonOp::Lte => left <= right,
        ComparisonOp::Eq => (left - right).abs() < f64::EPSILON,
    }
}

fn operand_value(operand: &OhlcvOperand, bars: &[OhlcvBar], index: usize) -> Option<f64> {
    match operand {
        OhlcvOperand::Field { field } => field_value(*field, &bars[index]),
        OhlcvOperand::Indicator { indicator, len } => {
            indicator_value(*indicator, *len, bars, index)
        }
        OhlcvOperand::Constant { value } if value.is_finite() => Some(*value),
        OhlcvOperand::Constant { .. } => None,
    }
}

fn field_value(field: OhlcvField, bar: &OhlcvBar) -> Option<f64> {
    match field {
        OhlcvField::Open => Some(bar.open),
        OhlcvField::High => Some(bar.high),
        OhlcvField::Low => Some(bar.low),
        OhlcvField::Close => Some(bar.close),
        OhlcvField::Volume => Some(bar.volume),
    }
}

fn indicator_value(
    indicator: OhlcvIndicator,
    len: Option<usize>,
    bars: &[OhlcvBar],
    index: usize,
) -> Option<f64> {
    match indicator {
        OhlcvIndicator::Sma => sma(bars, index, len?),
        OhlcvIndicator::PrevClose => index.checked_sub(1).map(|prev| bars[prev].close),
        OhlcvIndicator::ChangePct => change_pct(bars, index),
    }
}

fn validate_operand(operand: &OhlcvOperand) -> Result<(), CustomStrategyError> {
    if let OhlcvOperand::Indicator {
        indicator: OhlcvIndicator::Sma,
        len,
    } = operand
        && len.unwrap_or(0) == 0
    {
        return Err(CustomStrategyError::InvalidIndicatorLength);
    }
    Ok(())
}

fn sma(bars: &[OhlcvBar], index: usize, len: usize) -> Option<f64> {
    if len == 0 || index + 1 < len {
        return None;
    }
    let start = index + 1 - len;
    Some(bars[start..=index].iter().map(|bar| bar.close).sum::<f64>() / len as f64)
}

fn change_pct(bars: &[OhlcvBar], index: usize) -> Option<f64> {
    let previous = bars.get(index.checked_sub(1)?)?.close;
    if previous == 0.0 {
        None
    } else {
        Some((bars[index].close - previous) / previous * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_pairs_from_custom_conditions() {
        let strategy = CustomOhlcvStrategy {
            name: Some("breakout".into()),
            entry: vec![condition(ComparisonOp::Gt, 10.5)],
            exit: vec![condition(ComparisonOp::Lt, 11.5)],
            min_hold_bars: 1,
        };
        assert_eq!(
            custom_entry_exit_pairs(&strategy, &bars()).unwrap(),
            vec![(1, 3)]
        );
    }

    fn condition(op: ComparisonOp, value: f64) -> OhlcvCondition {
        OhlcvCondition {
            left: OhlcvOperand::Field {
                field: OhlcvField::Close,
            },
            op,
            right: OhlcvOperand::Constant { value },
        }
    }

    fn bars() -> Vec<OhlcvBar> {
        vec![
            bar("1", 10.0),
            bar("2", 11.0),
            bar("3", 12.0),
            bar("4", 11.0),
        ]
    }

    fn bar(timestamp: &str, close: f64) -> OhlcvBar {
        OhlcvBar {
            timestamp: timestamp.into(),
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1.0,
        }
    }
}
