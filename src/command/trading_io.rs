use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {label} {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {label} JSON {}", path.display()))
}

pub(crate) fn write_or_render<T: Serialize>(value: &T, out: Option<&Path>) -> Result<String> {
    let text = serde_json::to_string_pretty(value).context("failed to render JSON report")?;
    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create output dir {}", parent.display()))?;
        }
        std::fs::write(path, &text)
            .with_context(|| format!("failed to write report {}", path.display()))?;
        Ok(format!("Wrote Trading Lab report: {}", path.display()))
    } else {
        Ok(text)
    }
}
