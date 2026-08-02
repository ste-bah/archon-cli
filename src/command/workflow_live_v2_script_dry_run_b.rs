pub(super) fn dry_run_stub_result(method: WorkflowV2HostMethod) -> String {
    // The stub must carry the same envelope keys the live result view exposes
    // ({status, summary, data, result, ...}): reference-following scripts read
    // `x.result`/`x.data` fields, and a stub without them throws in the
    // pre-flight rehearsal, falsely rejecting a script that runs fine live.
    serde_json::json!({
        "status": "accepted",
        "summary": format!("dry-run stub result for w.{}", method.as_str()),
        "items": [],
        "outcomes": [],
        "data": {},
        "result": { "status": "accepted", "summary": "dry-run stub", "data": {} },
        "dry_run": true,
    })
    .to_string()
}

pub(super) fn artifact_requirements(value: &serde_json::Value) -> Vec<WorkflowV2ArtifactRequirement> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            serde_json::Value::String(path) => {
                let path = path.trim();
                (!path.is_empty()).then(|| WorkflowV2ArtifactRequirement::new(path))
            }
            serde_json::Value::Object(object) => {
                let path = object.get("path")?.as_str()?.trim();
                if path.is_empty() {
                    return None;
                }
                let mut requirement = WorkflowV2ArtifactRequirement::new(path);
                requirement.kind = object
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                Some(requirement)
            }
            _ => None,
        })
        .collect()
}
