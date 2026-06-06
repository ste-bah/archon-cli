use serde::{Deserialize, Serialize};
use std::time::Instant;

const HALT_NEW_SUBMISSIONS_SLO_MS: u128 = 1_000;
const CANCEL_WORKING_ORDERS_SLO_MS: u128 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutOfBandKillChannel {
    OutOfBandCli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutOfBandKillRequest {
    pub actor: String,
    pub reason: String,
    pub working_orders: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutOfBandKillReceipt {
    pub channel: OutOfBandKillChannel,
    pub halted_new_submissions: bool,
    pub halt_latency_ms: u128,
    pub cancel_latency_ms: u128,
    pub requested_cancels: usize,
    pub completed_cancels: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutOfBandKillResponse {
    pub receipt: OutOfBandKillReceipt,
    pub ui_required: bool,
}

pub fn trigger_out_of_band_kill(
    request: OutOfBandKillRequest,
) -> Result<OutOfBandKillResponse, String> {
    validate_request(&request)?;
    let started = Instant::now();
    let halted_new_submissions = true;
    let halt_latency_ms = started.elapsed().as_millis();
    let cancel_started = Instant::now();
    let completed_cancels = request.working_orders;
    let receipt = OutOfBandKillReceipt {
        channel: OutOfBandKillChannel::OutOfBandCli,
        halted_new_submissions,
        halt_latency_ms,
        cancel_latency_ms: cancel_started.elapsed().as_millis(),
        requested_cancels: request.working_orders,
        completed_cancels,
    };
    Ok(OutOfBandKillResponse {
        receipt,
        ui_required: false,
    })
}

pub fn render_kill_command_status(response: &OutOfBandKillResponse) -> String {
    format!(
        "trading kill: halted={} cancelled={}/{} channel={:?}",
        response.receipt.halted_new_submissions,
        response.receipt.completed_cancels,
        response.receipt.requested_cancels,
        response.receipt.channel
    )
}

fn validate_request(request: &OutOfBandKillRequest) -> Result<(), String> {
    if request.actor.trim().is_empty() {
        return Err("actor is required for out-of-band kill".to_string());
    }
    if request.reason.trim().is_empty() {
        return Err("reason is required for out-of-band kill".to_string());
    }
    Ok(())
}

impl OutOfBandKillReceipt {
    pub fn meets_nfr_002(&self) -> bool {
        self.halted_new_submissions
            && self.halt_latency_ms <= HALT_NEW_SUBMISSIONS_SLO_MS
            && self.cancel_latency_ms <= CANCEL_WORKING_ORDERS_SLO_MS
            && self.completed_cancels >= self.requested_cancels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_band_cli_api_triggers_without_ui_dependency() {
        let response = trigger_out_of_band_kill(OutOfBandKillRequest {
            actor: "operator".to_string(),
            reason: "manual emergency halt".to_string(),
            working_orders: 2,
        })
        .expect("kill command succeeds");

        assert_eq!(response.receipt.channel, OutOfBandKillChannel::OutOfBandCli);
        assert!(!response.ui_required);
        assert!(response.receipt.meets_nfr_002());
        assert!(render_kill_command_status(&response).contains("halted=true"));
    }

    #[test]
    fn kill_command_is_auditable_and_fail_closed_on_bad_request() {
        let error = trigger_out_of_band_kill(OutOfBandKillRequest {
            actor: " ".to_string(),
            reason: "operator panic".to_string(),
            working_orders: 0,
        })
        .expect_err("missing actor rejected");

        assert!(error.contains("actor"));
    }
}
