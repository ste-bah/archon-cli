//! Private, schema-checked reply reuse under the anchored evidence root.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    schema: u32,
    key: String,
    answer: serde_json::Value,
}
pub(super) fn key(
    client: &dyn WorkflowLlmClient,
    role: &str,
    prompt: &str,
    view: &BaseView,
    model: &str,
) -> String {
    let prompt = prompt.replace(&view.root().display().to_string(), "{view}");
    let objects: Vec<_> = view.files.iter().map(|(k, f)| (k, &f.object)).collect();
    content_digest(
        &serde_json::to_vec(&(
            BREAKER_LOGIC,
            role,
            prompt,
            objects,
            client.resolve_model_alias(model),
            client.provider_id(),
            client.request_identity(),
        ))
        .expect("cache identity"),
    )
}
pub(super) fn read(
    probe: &HostProbe,
    root: Option<&Path>,
    key: &str,
    refusals: &mut Vec<String>,
) -> Option<serde_json::Value> {
    let root = root.filter(|_| probe.resume.persist)?;
    let bytes = match std::fs::read(root.join("llm").join(format!("{key}.json"))) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            refusals.push(format!("llm_cache_unreadable: {key}"));
            return None;
        }
    };
    match serde_json::from_slice::<Cache>(&bytes) {
        Ok(c) if c.schema == VERDICT_SCHEMA && c.key == key => Some(c.answer),
        _ => {
            refusals.push(format!("llm_cache_unreadable: {key}"));
            None
        }
    }
}
pub(super) fn save(
    probe: &HostProbe,
    root: Option<&Path>,
    key: &str,
    answer: serde_json::Value,
    refusals: &mut Vec<String>,
) {
    let Some(root) = root.filter(|_| probe.resume.persist) else {
        return;
    };
    let result = serde_json::to_vec(&Cache {
        schema: VERDICT_SCHEMA,
        key: key.into(),
        answer,
    })
    .map_err(|e| e.to_string())
    .and_then(|b| super::super::store::atomic(&root.join("llm").join(format!("{key}.json")), &b));
    if result.is_ok() {
        probe.resume.progress.saved(true);
    } else {
        refusals.push(format!("llm_cache_not_saved: {key}"));
    }
}
