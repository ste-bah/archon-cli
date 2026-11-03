pub(crate) fn script_arguments_with_reply_markers(
    arguments: &serde_json::Value,
    markers: &[String],
) -> serde_json::Value {
    let mut script_arguments = arguments.clone();
    script_arguments["replyLeakedMarkers"] = serde_json::json!(markers);
    // Runtime site facts use the same args channel as reply markers, including resume.
    // Keep them out of launch identity: an old launch had no such prompt fact.
    script_arguments["acceptanceCheckSitePathLine"] = arguments["projectRoot"]
        .as_str()
        .zip(arguments["taskRoot"].as_str())
        .and_then(|(project, tasks)| {
            crate::command::acceptance_scratch_policy::check_site_path(
                std::path::Path::new(project),
                std::path::Path::new(tasks),
            )
        })
        .map(|path| crate::command::acceptance_scratch_policy::check_site_path_line(&path))
        .into();
    script_arguments
}

pub(crate) fn launch_metadata_arguments(arguments: &serde_json::Value) -> serde_json::Value {
    let mut durable = arguments.clone();
    if let Some(arguments) = durable.as_object_mut() {
        arguments.remove("replyLeakedMarkers");
        arguments.remove("acceptanceCheckSitePathLine");
    }
    durable
}

#[cfg(test)]
mod tests {
    use super::script_arguments_with_reply_markers;
    use serde_json::json;

    #[test]
    fn old_persisted_arguments_resume_with_empty_or_configured_reply_markers() {
        let persisted = json!({"taskRoot":"/project/tasks","frozenChain":{}});
        for markers in [Vec::new(), vec!["<marker>".to_string()]] {
            let runtime = script_arguments_with_reply_markers(&persisted, &markers);
            let compared = super::launch_metadata_arguments(&runtime);
            assert_eq!(compared, persisted);
            assert_eq!(runtime["replyLeakedMarkers"], json!(markers));
        }
    }
}
