use std::cmp::min;
use std::collections::BTreeMap;

use super::metadata::credential_state;

pub(super) struct OpenBbNativeRequest {
    pub(super) endpoint: &'static str,
    pub(super) params: BTreeMap<String, String>,
    pub(super) openbb_provider: String,
    pub(super) provider_symbol: String,
    pub(super) asset_class: String,
    pub(super) native_interval: String,
    pub(super) credential_state: BTreeMap<String, bool>,
}

pub(super) fn openbb_native_request(
    provider: &str,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    limit: u32,
) -> Result<OpenBbNativeRequest, String> {
    let openbb_provider = openbb_provider(provider)?;
    let endpoint = endpoint_for_symbol(symbol, &openbb_provider)?;
    let provider_symbol = provider_symbol_for(symbol, &openbb_provider);
    let native_interval = openbb_interval(timeframe)?;
    let mut params = BTreeMap::from([
        ("symbol".into(), provider_symbol.clone()),
        ("start_date".into(), date_part(start)),
        ("end_date".into(), date_part(end)),
        ("interval".into(), native_interval.clone()),
        ("sort".into(), "asc".into()),
        ("limit".into(), min(limit, 49_999).to_string()),
    ]);
    if endpoint != "/api/v1/derivatives/futures/historical" {
        params.insert("provider".into(), openbb_provider.clone());
    }
    Ok(OpenBbNativeRequest {
        endpoint,
        params,
        openbb_provider,
        provider_symbol,
        asset_class: asset_class(symbol).into(),
        native_interval,
        credential_state: credential_state(provider),
    })
}

pub(super) fn openbb_provider(provider: &str) -> Result<String, String> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openbb" | "polygon" => Ok("polygon".into()),
        "yfinance" => Ok("yfinance".into()),
        other => Err(format!(
            "{other} is not backed by the OpenBB native fetch path"
        )),
    }
}

fn endpoint_for_symbol(symbol: &str, openbb_provider: &str) -> Result<&'static str, String> {
    if is_future(symbol) {
        if openbb_provider == "yfinance" {
            return Ok("/api/v1/derivatives/futures/historical");
        }
        return Err("OpenBB/Polygon futures native fetch is unavailable; use TradingView MCP or an implemented futures provider".into());
    }
    if is_crypto(symbol) {
        return Ok("/api/v1/crypto/price/historical");
    }
    Ok("/api/v1/equity/price/historical")
}

fn provider_symbol_for(symbol: &str, openbb_provider: &str) -> String {
    match (symbol.trim().to_ascii_uppercase().as_str(), openbb_provider) {
        ("BTCUSDT", "yfinance") => "BTC-USD".into(),
        ("ETHUSDT", "yfinance") => "ETH-USD".into(),
        ("BTCUSDT", _) => "BTCUSD".into(),
        ("ETHUSDT", _) => "ETHUSD".into(),
        ("ES", "yfinance") => "ES=F".into(),
        ("NQ", "yfinance") => "NQ=F".into(),
        _ => symbol.trim().into(),
    }
}

fn openbb_interval(timeframe: &str) -> Result<String, String> {
    match timeframe.trim() {
        "1W" => Ok("1W".into()),
        "1D" => Ok("1d".into()),
        "240" | "4H" | "4h" => Ok("4h".into()),
        "60" | "1H" | "1h" => Ok("1h".into()),
        "15" | "15m" | "15M" => Ok("15m".into()),
        other => Err(format!("unsupported native timeframe `{other}`")),
    }
}

pub(super) fn date_part(value: &str) -> String {
    value.trim().chars().take(10).collect()
}

pub(super) fn is_crypto(symbol: &str) -> bool {
    matches!(
        symbol.trim().to_ascii_uppercase().as_str(),
        "BTCUSDT" | "ETHUSDT" | "BTCUSD" | "ETHUSD" | "BTC-USD" | "ETH-USD"
    )
}

pub(super) fn is_future(symbol: &str) -> bool {
    matches!(symbol.trim().to_ascii_uppercase().as_str(), "ES" | "NQ")
}

fn asset_class(symbol: &str) -> &'static str {
    if is_future(symbol) {
        "future"
    } else if is_crypto(symbol) {
        "crypto"
    } else {
        "equity"
    }
}
