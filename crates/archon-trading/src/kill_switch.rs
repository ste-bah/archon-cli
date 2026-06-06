use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub const HALT_NEW_SUBMISSIONS_SLO_MS: u128 = 1_000;
pub const CANCEL_WORKING_ORDERS_SLO_MS: u128 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KillChannel {
    InAppApi,
    OutOfBandCli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelReport {
    pub requested: usize,
    pub cancelled: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillReceipt {
    pub channel: KillChannel,
    pub halted_new_submissions: bool,
    pub halt_latency_ms: u128,
    pub cancel_latency_ms: u128,
    pub cancel_report: CancelReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillSwitchError {
    CancelTransport(String),
}

pub type CancelResult = Result<CancelReport, KillSwitchError>;
type CancelFn = Arc<dyn Fn() -> CancelResult + Send + Sync>;

#[derive(Clone)]
pub struct KillSwitch {
    halted: Arc<AtomicBool>,
    cancel_working_orders: CancelFn,
}

impl Default for KillSwitch {
    fn default() -> Self {
        Self::new(|| Ok(CancelReport::default()))
    }
}

impl Default for CancelReport {
    fn default() -> Self {
        Self {
            requested: 0,
            cancelled: 0,
        }
    }
}

impl KillSwitch {
    pub fn new(cancel_working_orders: impl Fn() -> CancelResult + Send + Sync + 'static) -> Self {
        Self {
            halted: Arc::new(AtomicBool::new(false)),
            cancel_working_orders: Arc::new(cancel_working_orders),
        }
    }

    pub fn trigger(&self) -> Result<KillReceipt, KillSwitchError> {
        self.trigger_from(KillChannel::InAppApi)
    }

    pub fn trigger_from(&self, channel: KillChannel) -> Result<KillReceipt, KillSwitchError> {
        let started = Instant::now();
        self.halted.store(true, Ordering::SeqCst);
        let halt_latency_ms = started.elapsed().as_millis();
        let cancel_started = Instant::now();
        let cancel_report = (self.cancel_working_orders)()?;
        Ok(KillReceipt {
            channel,
            halted_new_submissions: true,
            halt_latency_ms,
            cancel_latency_ms: cancel_started.elapsed().as_millis(),
            cancel_report,
        })
    }

    pub fn is_halted(&self) -> bool {
        self.halted.load(Ordering::SeqCst)
    }

    pub fn accepts_new_submissions(&self) -> bool {
        !self.is_halted()
    }
}

impl KillReceipt {
    pub fn meets_nfr_002(&self) -> bool {
        self.halted_new_submissions
            && self.halt_latency_ms <= HALT_NEW_SUBMISSIONS_SLO_MS
            && self.cancel_latency_ms <= CANCEL_WORKING_ORDERS_SLO_MS
            && self.cancel_report.cancelled >= self.cancel_report.requested
    }
}

impl std::fmt::Display for KillSwitchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CancelTransport(message) => {
                write!(formatter, "cancel transport failed: {message}")
            }
        }
    }
}

impl std::error::Error for KillSwitchError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn in_app_trigger_halts_and_cancels_within_nfr_002() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let switch = KillSwitch::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(CancelReport {
                requested: 3,
                cancelled: 3,
            })
        });

        let receipt = switch.trigger().expect("in-app kill succeeds");

        assert_eq!(receipt.channel, KillChannel::InAppApi);
        assert!(switch.is_halted());
        assert!(!switch.accepts_new_submissions());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(receipt.meets_nfr_002());
    }

    #[test]
    fn out_of_band_channel_independently_triggers_full_kill() {
        let switch = KillSwitch::new(|| {
            Ok(CancelReport {
                requested: 1,
                cancelled: 1,
            })
        });

        let receipt = switch
            .trigger_from(KillChannel::OutOfBandCli)
            .expect("out-of-band kill succeeds");

        assert_eq!(receipt.channel, KillChannel::OutOfBandCli);
        assert!(switch.is_halted());
        assert!(receipt.meets_nfr_002());
    }

    #[test]
    fn failed_cancel_transport_is_fail_closed_for_new_orders() {
        let switch = KillSwitch::new(|| Err(KillSwitchError::CancelTransport("down".to_string())));

        let result = switch.trigger();

        assert!(matches!(result, Err(KillSwitchError::CancelTransport(_))));
        assert!(switch.is_halted());
        assert!(!switch.accepts_new_submissions());
    }
}
