use serde_json::json;

pub(super) fn provider_environment_status(provider: &str) -> serde_json::Value {
    let normalized = provider.trim().to_ascii_lowercase();
    let keys = provider_env_keys(&normalized);
    json!({
        "provider": normalized,
        "redaction": "environment values omitted; key names and presence status only",
        "keys": keys
            .iter()
            .map(|key| json!({ "name": key, "status": env_status(key) }))
            .collect::<Vec<_>>()
    })
}

fn provider_env_keys(provider: &str) -> &'static [&'static str] {
    match provider {
        "openbb" | "polygon" => &["POLYGON_API_KEY", "OPENBB_API_URL"],
        "stooq" => &[
            "POLYGON_API_KEY",
            "OPENBB_API_KEY",
            "OPENBB_API_URL",
            "ARCHON_TRADINGVIEW_OHLCV_FIXTURE",
        ],
        "tradingview" => &["ARCHON_TRADINGVIEW_OHLCV_FIXTURE"],
        _ => &[],
    }
}

fn env_status(key: &str) -> &'static str {
    if std::env::var_os(key).is_some() {
        "present"
    } else {
        "missing"
    }
}
