use super::*;

pub(super) const PROVIDER_CAPABILITIES_SCHEMA: &str = "archon-provider-capabilities-v1";

pub(super) fn schema_artifact_value<T: Serialize>(
    value: &T,
) -> Result<serde_json::Value, DataStoreError> {
    let value = serde_json::to_value(value).map_err(json_error)?;
    let Some(mut object) = value.as_object().cloned() else {
        return Err(DataStoreError::InvalidMetadata(
            "schema artifact must serialize as an object".into(),
        ));
    };
    let schema = object
        .remove("schema")
        .or_else(|| object.remove("schema_version"))
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|schema| !schema.trim().is_empty())
        .ok_or_else(|| DataStoreError::InvalidMetadata("artifact schema is required".into()))?;
    canonicalize_dataset_status_casing(&schema, &mut object);
    object.insert("schema".into(), serde_json::Value::String(schema));
    if is_validation_report(&object) {
        add_validation_report_fields(&mut object);
    }
    Ok(serde_json::Value::Object(object))
}

fn canonicalize_dataset_status_casing(
    schema: &str,
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    if !matches!(schema, REGISTRY_SCHEMA_V1 | REGISTRY_SCHEMA_V2) {
        return;
    }
    canonicalize_status_field(object);
    if let Some(datasets) = object
        .get_mut("datasets")
        .and_then(serde_json::Value::as_object_mut)
    {
        for record in datasets
            .values_mut()
            .filter_map(serde_json::Value::as_object_mut)
        {
            canonicalize_status_field(record);
        }
    }
}

fn canonicalize_status_field(object: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(status) = object.get_mut("status") else {
        return;
    };
    let Some(raw) = status.as_str() else {
        return;
    };
    let canonical = if raw.eq_ignore_ascii_case("healthy") {
        Some("Healthy")
    } else if raw.eq_ignore_ascii_case("degraded") {
        Some("Degraded")
    } else {
        None
    };
    if let Some(canonical) = canonical {
        *status = serde_json::Value::String(canonical.to_string());
    }
}

fn is_validation_report(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    object.contains_key("dataset_id")
        && object.contains_key("version")
        && object.contains_key("checks")
        && object.contains_key("summary")
        && object.contains_key("validated_at")
}

fn add_validation_report_fields(object: &mut serde_json::Map<String, serde_json::Value>) {
    insert_check_field(
        object,
        "duplicate_timestamp_check",
        &["ohlcv.duplicate_timestamps"],
    );
    insert_check_field(
        object,
        "ohlc_check",
        &["ohlcv.ohlc_sanity", "ohlcv.valid_bars"],
    );
    insert_check_field(object, "volume_check", &["ohlcv.volume"]);
    insert_check_field(object, "gap_check", &["ohlcv.gaps"]);
    insert_check_field(
        object,
        "timestamp_check",
        &["ohlcv.rfc3339_timestamps", "ohlcv.monotonic_timestamps"],
    );
    insert_check_field(
        object,
        "metadata_check",
        &[
            "metadata.complete",
            "metadata.production_contract",
            "metadata.coverage_minimum",
            "metadata.native_interval",
            "metadata.not_derived_or_resampled",
            "metadata.production_eligible",
        ],
    );
}

fn insert_check_field(
    object: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    check_ids: &[&str],
) {
    let checks = matching_checks(object, check_ids);
    object.insert(
        field.into(),
        serde_json::json!({
            "status": aggregate_check_status(&checks),
            "check_ids": check_ids,
            "checks": checks,
        }),
    );
}

fn matching_checks(
    object: &serde_json::Map<String, serde_json::Value>,
    check_ids: &[&str],
) -> Vec<serde_json::Value> {
    object
        .get("checks")
        .and_then(serde_json::Value::as_array)
        .map(|checks| {
            checks
                .iter()
                .filter(|check| {
                    check
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|id| check_ids.contains(&id))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn aggregate_check_status(checks: &[serde_json::Value]) -> &'static str {
    if checks
        .iter()
        .any(|check| check_status(check) == Some("failed"))
    {
        "failed"
    } else if checks.is_empty() {
        "missing"
    } else if checks
        .iter()
        .any(|check| check_status(check) == Some("degraded"))
    {
        "degraded"
    } else {
        "passed"
    }
}

fn check_status(check: &serde_json::Value) -> Option<&str> {
    check.get("status").and_then(serde_json::Value::as_str)
}

pub(super) fn write_schema_json<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), DataStoreError> {
    write_json(path, &schema_artifact_value(value)?)
}

pub(super) fn write_schema_json_with_backup<T: Serialize>(
    path: &Path,
    value: &T,
    backup_path: &Path,
) -> Result<(), DataStoreError> {
    write_json_with_backup(path, &schema_artifact_value(value)?, backup_path)
}

fn json_error(error: serde_json::Error) -> DataStoreError {
    DataStoreError::Json(error.to_string())
}
