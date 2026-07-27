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
        self.write_capabilities(&records, &result.checked_at)?;
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
        self.write_capabilities(&records, &result.checked_at)?;
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
            "schema_version": PROVIDER_CAPABILITIES_SCHEMA,
            "checked_at": result.checked_at,
            "capability": result,
            "provider_environment": provider_environment_status(&result.provider),
        });
        write_schema_json(&self.provider_capability_latest_path(), &artifact)
    }

    fn write_capabilities(
        &self,
        records: &BTreeMap<String, ProviderCapabilityResult>,
        checked_at: &str,
    ) -> Result<(), DataStoreError> {
        let artifact = serde_json::json!({
            "schema_version": PROVIDER_CAPABILITIES_SCHEMA,
            "checked_at": checked_at,
            "capabilities": enriched_capability_records(records),
        });
        write_schema_json(&self.provider_capabilities_path(), &artifact)
    }

    pub fn load_capabilities(
        &self,
    ) -> Result<BTreeMap<String, ProviderCapabilityResult>, DataStoreError> {
        let path = self.provider_capabilities_path();
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        let value: serde_json::Value = read_json(&path)?;
        let mut records = capability_records(value)?;
        backfill_legacy_capability_symbols(&mut records);
        serde_json::from_value(records).map_err(|err| DataStoreError::Json(err.to_string()))
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
        let path = self.snapshot_path(&snapshot.provider, &snapshot.canonical_instrument);
        let artifact = serde_json::json!({
            "schema_version": "archon-trading-snapshot-v1",
            "snapshot": snapshot,
            "freshness": freshness,
            "classified_at_unix_seconds": now_unix_seconds
        });
        let artifact = schema_artifact_value(&artifact)?;
        write_json(&path, &artifact)?;
        self.record_snapshot_artifact(
            &snapshot.provider,
            &snapshot.canonical_instrument,
            &artifact,
        )?;
        Ok(path)
    }

    fn record_snapshot_artifact(
        &self,
        provider: &str,
        instrument: &str,
        artifact: &serde_json::Value,
    ) -> Result<(), DataStoreError> {
        let mut registry = self.load_registry()?;
        registry.snapshots.insert(
            capability_key(provider, instrument, "snapshot"),
            artifact.clone(),
        );
        write_schema_json(&self.registry_path(), &registry)
    }
}

fn capability_records(value: serde_json::Value) -> Result<serde_json::Value, DataStoreError> {
    let Some(object) = value.as_object() else {
        return Err(DataStoreError::Json(
            "provider capabilities artifact must be an object".into(),
        ));
    };
    if let Some(records) = object.get("capabilities") {
        let schema = object.get("schema").and_then(serde_json::Value::as_str);
        if schema != Some(PROVIDER_CAPABILITIES_SCHEMA) {
            return Err(DataStoreError::InvalidMetadata(
                "unsupported provider capabilities schema".into(),
            ));
        }
        return Ok(records.clone());
    }
    Ok(value)
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
    let provider_implemented = result.unavailable_reason.is_none();
    if result.can_fetch
        && (!result.native_interval
            || !result.historical_supported
            || !result.production_eligible
            || result.missing_credentials
            || result.provider_blocked
            || result.unsupported)
    {
        return Err(DataStoreError::InvalidMetadata(
            "can_fetch=true requires proven native_interval, historical_supported, production_eligible, credentials, unblocked provider support, and provider implementation".into(),
        ));
    }
    if result.can_fetch && !provider_implemented {
        return Err(DataStoreError::InvalidMetadata(
            "can_fetch=true requires downstream provider implementation proof, not capability metadata alone".into(),
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

fn enriched_capability_records(
    records: &BTreeMap<String, ProviderCapabilityResult>,
) -> BTreeMap<String, serde_json::Value> {
    records
        .iter()
        .map(|(key, result)| (key.clone(), enriched_capability_record(result)))
        .collect()
}

fn enriched_capability_record(result: &ProviderCapabilityResult) -> serde_json::Value {
    let mut value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    object.insert(
        "capability_state".into(),
        serde_json::Value::String(capability_state(result).into()),
    );
    object.insert(
        "provider_env_proof".into(),
        provider_env_proof(&result.provider),
    );
    object.insert("registry_backed".into(), serde_json::Value::Bool(false));
    object.insert(
        "registry_status".into(),
        serde_json::Value::String(registry_status(result).into()),
    );
    object.insert(
        "registry_dataset_id".into(),
        registry_dataset_id(result).map_or(serde_json::Value::Null, serde_json::Value::String),
    );
    object.insert(
        "registry_version".into(),
        registry_version(result).map_or(serde_json::Value::Null, serde_json::Value::String),
    );
    object.insert("registry_key".into(), serde_json::Value::Null);
    object.insert(
        "fail_closed_behavior".into(),
        serde_json::Value::String(
            "unavailable capability probes persist proof only and do not write production dataset registry entries".into(),
        ),
    );
    object.insert(
        "validation".into(),
        serde_json::json!({"status": if result.can_fetch { "passed" } else { "failed_closed" }}),
    );
    if result.provider == "provider_state" && result.timeframe == "snapshot" {
        object.insert(
            "snapshot_freshness".into(),
            serde_json::Value::String(snapshot_freshness_label(result).into()),
        );
    }
    value
}

fn capability_state(result: &ProviderCapabilityResult) -> &'static str {
    if result.provider == "provider_state" && result.timeframe == "snapshot" {
        return snapshot_freshness_label(result);
    }
    if result.can_fetch {
        "can_fetch"
    } else if result.provider_blocked {
        "provider_blocked"
    } else if result.missing_credentials {
        "missing_credentials"
    } else if result.unsupported {
        "unsupported"
    } else {
        "unavailable"
    }
}

fn registry_status(result: &ProviderCapabilityResult) -> &'static str {
    if result.can_fetch {
        "Available"
    } else {
        "Unavailable"
    }
}

fn registry_dataset_id(result: &ProviderCapabilityResult) -> Option<String> {
    result.can_fetch.then(|| {
        format!(
            "{}:{}:{}",
            result.provider, result.canonical_instrument, result.timeframe
        )
    })
}

fn registry_version(result: &ProviderCapabilityResult) -> Option<String> {
    result.can_fetch.then(|| result.checked_at.clone())
}

fn snapshot_freshness_label(result: &ProviderCapabilityResult) -> &'static str {
    if result
        .unavailable_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("stale"))
    {
        "stale"
    } else {
        "missing"
    }
}

fn provider_env_proof(provider: &str) -> serde_json::Value {
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
    let checked_keys = keys
        .iter()
        .map(|key| serde_json::json!({"name": key, "presence": env_presence(key)}))
        .collect::<Vec<_>>();
    serde_json::json!({
        "checked_keys": checked_keys,
        "credential_state": env_credential_state(keys),
        "redaction": "credential values omitted; only key names and presence states are recorded"
    })
}

fn env_presence(key: &str) -> &'static str {
    if std::env::var_os(key).is_some() {
        "present"
    } else {
        "missing"
    }
}

fn env_credential_state(keys: &[&str]) -> &'static str {
    if keys.is_empty() {
        return "not_required";
    }
    if keys.iter().any(|key| std::env::var_os(key).is_some()) {
        "present"
    } else {
        "missing"
    }
}
