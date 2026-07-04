use super::*;

impl TradingDataLake {
    pub fn persist_capability(
        &self,
        provider: &str,
        symbol: &str,
        timeframe: &str,
        checked_at: &str,
    ) -> Result<ProviderCapabilityResult, DataStoreError> {
        let result = can_fetch_symbol_timeframe(provider, symbol, timeframe, checked_at);
        let mut records = self.load_capabilities()?;
        records.insert(capability_key(provider, symbol, timeframe), result.clone());
        write_json(&self.provider_capabilities_path(), &records)?;
        let persisted = self.load_capabilities()?;
        verify_persisted_capability(persisted.get(&capability_key(provider, symbol, timeframe)))?;
        self.write_latest_capability(&result)?;
        Ok(result)
    }

    pub fn persist_capability_result(
        &self,
        result: ProviderCapabilityResult,
    ) -> Result<ProviderCapabilityResult, DataStoreError> {
        verify_capability_contract(&result)?;
        let mut records = self.load_capabilities()?;
        let key = capability_key(
            &result.provider,
            &result.canonical_instrument,
            &result.timeframe,
        );
        records.insert(key.clone(), result.clone());
        write_json(&self.provider_capabilities_path(), &records)?;
        let persisted = self.load_capabilities()?;
        verify_persisted_capability(persisted.get(&key))?;
        self.write_latest_capability(&result)?;
        Ok(result)
    }

    fn write_latest_capability(
        &self,
        result: &ProviderCapabilityResult,
    ) -> Result<(), DataStoreError> {
        verify_capability_contract(result)?;
        let artifact = serde_json::json!({
            "capability": result,
            "provider_environment": provider_environment_status(&result.provider),
        });
        write_json(&self.provider_capability_latest_path(), &artifact)
    }

    pub fn load_capabilities(
        &self,
    ) -> Result<BTreeMap<String, ProviderCapabilityResult>, DataStoreError> {
        let path = self.provider_capabilities_path();
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        let mut value: serde_json::Value = read_json(&path)?;
        backfill_legacy_capability_symbols(&mut value);
        serde_json::from_value(value).map_err(|err| DataStoreError::Json(err.to_string()))
    }

    pub fn persist_snapshot(
        &self,
        snapshot: crate::data_lake::CurrentSnapshot,
        now_unix_seconds: i64,
    ) -> Result<PathBuf, DataStoreError> {
        let freshness = crate::data_lake::snapshot_freshness(
            Some(snapshot.captured_at_unix_seconds),
            now_unix_seconds,
        );
        let path = self
            .snapshot_dir(&snapshot.provider, &snapshot.canonical_instrument)
            .join(format!("{}.json", snapshot.captured_at_unix_seconds));
        let artifact = serde_json::json!({
            "schema_version": "archon-trading-snapshot-v1",
            "snapshot": snapshot,
            "freshness": freshness,
            "classified_at_unix_seconds": now_unix_seconds
        });
        write_json(&path, &artifact)?;
        Ok(path)
    }
}

fn backfill_legacy_capability_symbols(value: &mut serde_json::Value) {
    let Some(records) = value.as_object_mut() else {
        return;
    };
    for record in records.values_mut() {
        let Some(record) = record.as_object_mut() else {
            continue;
        };
        if record.contains_key("symbol") {
            continue;
        }
        if let Some(symbol) = record.get("canonical_instrument").cloned() {
            record.insert("symbol".into(), symbol);
        }
    }
}

fn verify_persisted_capability(
    result: Option<&ProviderCapabilityResult>,
) -> Result<(), DataStoreError> {
    let Some(result) = result else {
        return Err(DataStoreError::Json(
            "capability persistence verification failed".into(),
        ));
    };
    verify_capability_contract(result)
}

fn verify_capability_contract(result: &ProviderCapabilityResult) -> Result<(), DataStoreError> {
    if result.can_fetch
        && (!result.native_interval || !result.historical_supported || !result.production_eligible)
    {
        return Err(DataStoreError::InvalidMetadata(
            "can_fetch=true requires proven native_interval, historical_supported, production_eligible, and provider implementation".into(),
        ));
    }
    if result.can_fetch && result.unavailable_reason.is_some() {
        return Err(DataStoreError::InvalidMetadata(
            "can_fetch=true cannot include unavailable_reason".into(),
        ));
    }
    Ok(())
}

fn provider_environment_status(provider: &str) -> serde_json::Value {
    let normalized = provider.trim().to_ascii_lowercase();
    let keys: &[&str] = match normalized.as_str() {
        "openbb" | "polygon" => &["POLYGON_API_KEY", "OPENBB_API_URL"],
        "stooq" => &[
            "POLYGON_API_KEY",
            "OPENBB_API_KEY",
            "OPENBB_API_URL",
            "ARCHON_TRADINGVIEW_OHLCV_FIXTURE",
        ],
        "tradingview" => &["ARCHON_TRADINGVIEW_OHLCV_FIXTURE"],
        _ => &[],
    };
    serde_json::json!({
        "provider": normalized,
        "redaction": "environment values omitted; key names and presence status only",
        "keys": keys.iter().map(|key| {
            serde_json::json!({
                "name": key,
                "status": env_status(key),
            })
        }).collect::<Vec<_>>()
    })
}

fn env_status(key: &str) -> &'static str {
    if std::env::var_os(key).is_some() {
        "present"
    } else {
        "missing"
    }
}
