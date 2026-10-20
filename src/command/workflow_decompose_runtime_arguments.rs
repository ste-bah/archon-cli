pub(crate) fn script_arguments_with_reply_markers(
    arguments: &serde_json::Value,
    markers: &[String],
) -> serde_json::Value {
    let mut script_arguments = arguments.clone();
    script_arguments["replyLeakedMarkers"] = serde_json::json!(markers);
    script_arguments
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
            let mut compared = runtime.clone();
            compared
                .as_object_mut()
                .unwrap()
                .remove("replyLeakedMarkers");
            assert_eq!(compared, persisted);
            assert_eq!(runtime["replyLeakedMarkers"], json!(markers));
        }
    }
}
