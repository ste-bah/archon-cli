use anyhow::Result;
use archon_trading::promotion::{PromotionEvidence, evaluate_promotion};
use archon_trading::spec_registry::{PromotionStatus, StrategySpec};
use serde_json::json;

use crate::cli_args::{TradingCliPromoteAction, TradingCliPromotionStatus};
use crate::command::trading_io::{read_json, write_or_render};

pub(crate) fn render_promote(action: &TradingCliPromoteAction) -> Result<String> {
    match action {
        TradingCliPromoteAction::Check {
            spec,
            target,
            evidence,
            paper_sample,
            postmortem,
            out,
        } => {
            let spec: StrategySpec = read_json(spec, "StrategySpec")?;
            let evidence: Vec<PromotionEvidence> = read_json(evidence, "PromotionEvidence[]")?;
            let paper_sample = read_optional(paper_sample.as_deref(), "PaperSample")?;
            let postmortem = read_optional(postmortem.as_deref(), "SessionPostmortem")?;
            let result = evaluate_promotion(
                &spec,
                (*target).into(),
                &evidence,
                paper_sample.as_ref(),
                postmortem.as_ref(),
            );
            let report = match result {
                Ok(report) => json!({"accepted": true, "report": report}),
                Err(err) => json!({"accepted": false, "error": format!("{err:?}")}),
            };
            write_or_render(&report, out.as_deref())
        }
    }
}

fn read_optional<T>(path: Option<&std::path::Path>, label: &str) -> Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    path.map(|path| read_json(path, label)).transpose()
}

impl From<TradingCliPromotionStatus> for PromotionStatus {
    fn from(value: TradingCliPromotionStatus) -> Self {
        match value {
            TradingCliPromotionStatus::Idea => Self::Idea,
            TradingCliPromotionStatus::Research => Self::Research,
            TradingCliPromotionStatus::Backtest => Self::Backtest,
            TradingCliPromotionStatus::Paper => Self::Paper,
            TradingCliPromotionStatus::LivePilot => Self::LivePilot,
            TradingCliPromotionStatus::Retired => Self::Retired,
        }
    }
}
