fn sanitize_call_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> WorkflowResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkflowError::io(parent, err))?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|err| WorkflowError::io(&tmp, err))?;
    fs::rename(&tmp, path).map_err(|err| WorkflowError::io(path, err))
}

fn sanitize_for_persistence<T>(value: &T) -> WorkflowResult<T>
where
    T: Serialize + DeserializeOwned,
{
    let value = serde_json::to_value(value)?;
    serde_json::from_value(sanitize_value(value)).map_err(Into::into)
}
