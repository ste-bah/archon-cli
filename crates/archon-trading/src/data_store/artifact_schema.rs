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
    object.insert("schema".into(), serde_json::Value::String(schema));
    Ok(serde_json::Value::Object(object))
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
