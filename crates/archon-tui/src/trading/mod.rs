use archon_trading::audit_ledger::OrderStatus;
use archon_trading::kill_switch::{KillReceipt, KillSwitch, KillSwitchError};
use archon_trading::order_intent::TradingMode;
use archon_trading::risk_governor::{RiskDecision, RiskDecisionStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use serde::Serialize;

use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TradingStatusView {
    pub live_enabled: bool,
    pub kill_switch_halted: bool,
    pub active_strategy_count: usize,
    pub pending_order_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionLedgerRow {
    pub order_id: String,
    pub strategy_id: String,
    pub status: OrderStatus,
    pub mode: TradingMode,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RiskDecisionRow {
    pub strategy_id: String,
    pub decision: RiskDecision,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TradingPanelState {
    pub status: TradingStatusView,
    pub ledger: Vec<ExecutionLedgerRow>,
    pub risk_decisions: Vec<RiskDecisionRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillButtonError {
    Engine(KillSwitchError),
}

impl Default for TradingPanelState {
    fn default() -> Self {
        Self {
            status: TradingStatusView {
                live_enabled: false,
                kill_switch_halted: false,
                active_strategy_count: 0,
                pending_order_count: 0,
            },
            ledger: Vec::new(),
            risk_decisions: Vec::new(),
        }
    }
}

impl TradingPanelState {
    pub fn status_lines(&self) -> Vec<String> {
        vec![
            format!("live enabled: {}", self.status.live_enabled),
            format!("kill switch halted: {}", self.status.kill_switch_halted),
            format!("active strategies: {}", self.status.active_strategy_count),
            format!("pending orders: {}", self.status.pending_order_count),
        ]
    }

    pub fn ledger_lines(&self) -> Vec<String> {
        self.ledger
            .iter()
            .map(|row| {
                format!(
                    "{} {} {:?} {:?} {}",
                    row.order_id, row.strategy_id, row.mode, row.status, row.note
                )
            })
            .collect()
    }

    pub fn risk_lines(&self) -> Vec<String> {
        self.risk_decisions
            .iter()
            .map(|row| risk_line(&row.strategy_id, &row.decision))
            .collect()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(40),
                Constraint::Percentage(30),
            ])
            .split(area);
        frame.render_widget(status_widget(self, theme), columns[0]);
        frame.render_widget(
            lines_widget("Execution Ledger", self.ledger_lines(), theme),
            columns[1],
        );
        frame.render_widget(
            lines_widget("Risk Decisions", self.risk_lines(), theme),
            columns[2],
        );
    }
}

pub fn trigger_kill_button(kill_switch: &KillSwitch) -> Result<KillReceipt, KillButtonError> {
    kill_switch.trigger().map_err(KillButtonError::Engine)
}

fn status_widget<'a>(state: &TradingPanelState, theme: &Theme) -> Paragraph<'a> {
    let mut lines = state
        .status_lines()
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "[K] ",
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Kill Switch"),
    ]));
    Paragraph::new(lines).block(block("Trading Status", theme))
}

fn lines_widget<'a>(title: &'a str, rows: Vec<String>, theme: &Theme) -> List<'a> {
    let items = if rows.is_empty() {
        vec![ListItem::new("no records")]
    } else {
        rows.into_iter().map(ListItem::new).collect()
    };
    List::new(items).block(block(title, theme))
}

fn block<'a>(title: &'a str, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(theme.border))
}

fn risk_line(strategy_id: &str, decision: &RiskDecision) -> String {
    let status = match decision.status {
        RiskDecisionStatus::Approved => "approved",
        RiskDecisionStatus::Rejected => "rejected",
        RiskDecisionStatus::Halted => "halted",
        RiskDecisionStatus::Retired => "retired",
    };
    let control = decision.control_id.unwrap_or("none");
    format!(
        "{strategy_id} {status} control={control} {}ms",
        decision.latency_ms
    )
}

impl std::fmt::Display for KillButtonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Engine(error) => write!(formatter, "kill switch trigger failed: {error}"),
        }
    }
}
impl std::error::Error for KillButtonError {}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_trading::kill_switch::{CancelReport, KillChannel};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn panel_render_model_includes_status_ledger_and_risk() {
        let state = TradingPanelState {
            status: TradingStatusView {
                live_enabled: false,
                kill_switch_halted: false,
                active_strategy_count: 1,
                pending_order_count: 2,
            },
            ledger: vec![ExecutionLedgerRow {
                order_id: "paper-1".to_string(),
                strategy_id: "strat-a".to_string(),
                status: OrderStatus::Accepted,
                mode: TradingMode::Paper,
                note: "accepted".to_string(),
            }],
            risk_decisions: vec![RiskDecisionRow {
                strategy_id: "strat-a".to_string(),
                decision: RiskDecision {
                    status: RiskDecisionStatus::Approved,
                    control_id: None,
                    attribution: None,
                    terminal: false,
                    recoverable: false,
                    latency_ms: 3,
                },
            }],
        };

        assert!(
            state
                .status_lines()
                .iter()
                .any(|line| line == "active strategies: 1")
        );
        assert!(state.ledger_lines()[0].contains("paper-1 strat-a Paper Accepted"));
        assert_eq!(state.risk_lines()[0], "strat-a approved control=none 3ms");
    }

    #[test]
    fn kill_button_invokes_existing_in_app_trigger_api() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let switch = KillSwitch::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(CancelReport {
                requested: 1,
                cancelled: 1,
            })
        });

        let receipt = trigger_kill_button(&switch).expect("kill button triggers channel 2");

        assert_eq!(receipt.channel, KillChannel::InAppApi);
        assert!(switch.is_halted());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
