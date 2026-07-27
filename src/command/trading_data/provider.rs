use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use archon_trading::data_store::TradingDataLake;

use crate::command::trading_io::write_or_render;
use crate::command::trading_tools::project_root;

use super::{data_error, trading_data_env};

pub(super) fn providers(target: Option<&PathBuf>) -> Result<String> {
    let root = project_root(target)?;
    let lake = TradingDataLake::new(root);
    let report = json!({
        "providers": ["tradingview", "openbb", "polygon", "stooq", "yfinance"],
        "capabilities_path": lake.provider_capabilities_path(),
        "fetch_contract": "provider-neutral; provider-specific fetches fail closed until implemented"
    });
    write_or_render(&report, None)
}

pub(super) fn capability(
    target: Option<&PathBuf>,
    provider: &str,
    symbol: &str,
    timeframe: &str,
) -> Result<String> {
    let root = project_root(target)?;
    let provider_key = provider.trim().to_ascii_lowercase();
    let lake = TradingDataLake::new(root);
    let result = if matches!(provider_key.as_str(), "openbb" | "polygon" | "yfinance") {
        let base_url =
            std::env::var("OPENBB_API_URL").unwrap_or_else(|_| "http://127.0.0.1:6900".into());
        super::super::trading_data_provider_openbb::probe_capability_with_base_url(
            lake.project_root(),
            &base_url,
            provider,
            symbol,
            timeframe,
        )?
    } else {
        lake.persist_capability(
            provider,
            symbol,
            timeframe,
            &chrono::Utc::now().to_rfc3339(),
        )
        .map_err(data_error)?
    };
    let exact_native_support = result.native_interval && result.historical_supported;
    let capability_state = if result.can_fetch {
        "can_fetch"
    } else if result.provider_blocked {
        "provider_blocked"
    } else if result.native_interval && !result.production_eligible {
        "degraded"
    } else {
        "unavailable"
    };
    let mut capability_states = vec![capability_state];
    if exact_native_support {
        capability_states.push("exact_native_support");
    }
    let report = json!({
        "provider": result.provider,
        "symbol": result.symbol,
        "canonical_instrument": result.canonical_instrument,
        "provider_symbol": result.provider_symbol,
        "timeframe": result.timeframe,
        "native_interval": result.native_interval,
        "production_eligible": result.production_eligible,
        "can_fetch": result.can_fetch,
        "capability_state": capability_state,
        "capability_states": capability_states,
        "exact_native_support": exact_native_support,
        "current_snapshot_supported": result.current_snapshot_supported,
        "historical_supported": result.historical_supported,
        "history_horizon": result.history_horizon,
        "requires_credentials": result.requires_credentials,
        "missing_credentials": result.missing_credentials,
        "provider_blocked": result.provider_blocked,
        "unsupported": result.unsupported,
        "credential_state": result.credential_state,
        "unavailable_reason": result.unavailable_reason,
        "checked_at": result.checked_at,
        "provider_environment": trading_data_env::provider_environment_status(provider),
        "capability_artifact": lake.provider_capability_latest_path(),
        "fail_closed_behavior": "unavailable capability probes persist proof only and do not write production dataset registry entries"
    });
    write_or_render(&report, None)
}
