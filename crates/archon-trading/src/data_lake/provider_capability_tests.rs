use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct TestAdapter {
    probes: Arc<AtomicUsize>,
    histories: Arc<AtomicUsize>,
    snapshots: Arc<AtomicUsize>,
    snapshot_supported: bool,
}

impl ProviderAdapter for TestAdapter {
    fn provider_id(&self) -> &str {
        "mock"
    }

    fn probe_capability(
        &self,
        request: &CapabilityRequest,
        budget: ProbeBudget,
    ) -> Result<CapabilityEvidence, UnavailableReason> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        assert_eq!(budget, ProbeBudget::default());
        Ok(CapabilityEvidence {
            provider: request.provider.clone(),
            canonical_instrument: request.canonical_instrument.clone(),
            provider_symbol: request.provider_symbol.clone(),
            timeframe: request.timeframe.clone(),
            native_interval: true,
            historical_supported: true,
            current_snapshot_supported: self.snapshot_supported,
            requires_credentials: false,
            credential_available: true,
            history_horizon: None,
            candles_examined: 2,
            response_bytes: 1_024,
        })
    }

    fn fetch_native_history(
        &self,
        request: &NativeFetchRequest,
    ) -> Result<NativeHistory, UnavailableReason> {
        self.histories.fetch_add(1, Ordering::SeqCst);
        Ok(NativeHistory {
            provider: request.provider.clone(),
            canonical_instrument: request.canonical_instrument.clone(),
            provider_symbol: request.provider_symbol.clone(),
            timeframe: request.timeframe.clone(),
            requested_start: request.start.clone(),
            requested_end: request.end.clone(),
            coverage_start: request.start.clone(),
            coverage_end: request.end.clone(),
            provenance: NativeProvenance {
                provider: request.provider.clone(),
                provider_symbol: request.provider_symbol.clone(),
                native_timeframe: request.timeframe.clone(),
                resampled: false,
            },
            bars: vec![crate::ohlcv::OhlcvBar {
                timestamp: request.start.clone(),
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                volume: 10.0,
            }],
        })
    }

    fn supports_current_snapshot(&self) -> bool {
        self.snapshot_supported
    }

    fn fetch_current_snapshot(
        &self,
        request: &SnapshotRequest,
    ) -> Result<CurrentSnapshot, UnavailableReason> {
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        Ok(CurrentSnapshot {
            provider: request.provider.clone(),
            canonical_instrument: request.canonical_instrument.clone(),
            provider_symbol: request.provider_symbol.clone(),
            captured_at_unix_seconds: 1,
            payload: serde_json::json!({"price": 1.5}),
        })
    }
}

fn adapter(snapshot_supported: bool) -> (TestAdapter, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let histories = Arc::new(AtomicUsize::new(0));
    let snapshots = Arc::new(AtomicUsize::new(0));
    (
        TestAdapter {
            probes: Arc::new(AtomicUsize::new(0)),
            histories: histories.clone(),
            snapshots: snapshots.clone(),
            snapshot_supported,
        },
        histories,
        snapshots,
    )
}

fn capability_request(provider: &str) -> CapabilityRequest {
    CapabilityRequest {
        provider: provider.into(),
        canonical_instrument: "SPY".into(),
        provider_symbol: "SPY.US".into(),
        timeframe: "1D".into(),
        checked_at: "2026-01-01T00:00:00Z".into(),
    }
}

#[test]
fn provider_capability_result_fails_closed() {
    let result = ProviderDispatcher::new().can_fetch_symbol_timeframe(&capability_request("none"));
    assert!(!result.can_fetch);
    assert!(!result.production_eligible);
    assert_eq!(
        result.unavailable_reason.as_deref(),
        Some("provider_not_registered")
    );
}

#[test]
fn provider_capability_never_calls_full_history() {
    let (adapter, histories, _) = adapter(false);
    let mut dispatcher = ProviderDispatcher::new();
    dispatcher.register(adapter);
    let result = dispatcher.can_fetch_symbol_timeframe(&capability_request("mock"));
    assert!(result.can_fetch);
    assert!(!result.production_eligible);
    assert_eq!(histories.load(Ordering::SeqCst), 0);
}

#[test]
fn provider_native_fetch_preserves_identity_and_provenance() {
    let (adapter, _, _) = adapter(false);
    let mut dispatcher = ProviderDispatcher::new();
    dispatcher.register(adapter);
    let request = NativeFetchRequest {
        provider: "mock".into(),
        canonical_instrument: "SPY".into(),
        provider_symbol: "SPY.US".into(),
        timeframe: "1D".into(),
        start: "2026-01-01T00:00:00Z".into(),
        end: "2026-01-02T00:00:00Z".into(),
    };
    let NativeFetchResult::Complete { history } = dispatcher.fetch_ohlcv_native(&request) else {
        panic!("expected complete exact-native history");
    };
    assert_eq!(history.provider, request.provider);
    assert_eq!(history.provider_symbol, request.provider_symbol);
    assert_eq!(history.provenance.native_timeframe, request.timeframe);
    assert!(!history.provenance.resampled);
}

#[test]
fn provider_snapshot_requires_explicit_support() {
    let (adapter, _, snapshots) = adapter(false);
    let mut dispatcher = ProviderDispatcher::new();
    dispatcher.register(adapter);
    let result = dispatcher.fetch_current_snapshot(&SnapshotRequest {
        provider: "mock".into(),
        canonical_instrument: "SPY".into(),
        provider_symbol: "SPY.US".into(),
    });
    assert_eq!(
        result,
        SnapshotResult::Unavailable {
            reason: UnavailableReason::CurrentSnapshotUnsupported
        }
    );
    assert_eq!(snapshots.load(Ordering::SeqCst), 0);
}
