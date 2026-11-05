//! Issue 601: carry body judgments so JavaScript can apply its own ranking.
use super::*;

pub(super) fn collect(
    subject: &str,
    records: &[&WorkflowV2CallRecord],
    gates: &[Gate<'_>],
) -> Vec<Value> {
    if !subject.starts_with("body-") {
        return Vec::new();
    }
    let mut replies: Vec<_> = records
        .iter()
        .filter_map(|record| {
            let (owner, entry, ordinal) = author_call(&record.call.id)?;
            (owner == subject && entry.is_none()).then_some((ordinal, *record))
        })
        .collect();
    replies.sort_by_key(|(ordinal, _)| *ordinal);
    replies
        .into_iter()
        .filter_map(|(_, record)| {
            let body = reply_content(record)?;
            let gate = gates
                .iter()
                .rfind(|(_, command, stdin, _)| *command == "land-task-body" && *stdin == body)?;
            Some(serde_json::json!({ "body": body, "findings": gate.3.findings }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_results_keep_each_body_with_its_latest_judgment_in_author_order() {
        let older = super::super::tests::reply("body-TASK-1-author-1", "02:00:00", "older body");
        let newer = super::super::tests::reply("body-TASK-1-author-2", "01:00:00", "newer body");
        let unrelated =
            super::super::tests::reply("body-TASK-2-author-1", "03:00:00", "other body");
        let records = vec![&newer, &older, &unrelated];
        let judged = |text: &str| GateSeed {
            call_id: text.into(),
            published: false,
            findings: vec![serde_json::json!({"text": text, "remediation_scope": "body"})],
        };
        let gates = vec![
            (&older, "land-task-body", "older body", judged("stale")),
            (&newer, "land-task-body", "newer body", judged("newer")),
            (&older, "land-task-body", "older body", judged("latest")),
        ];
        let results = collect("body-TASK-1", &records, &gates);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["body"], "older body");
        assert_eq!(results[0]["findings"][0]["text"], "latest");
        assert_eq!(results[1]["body"], "newer body");
        assert!(collect("skeleton", &records, &gates).is_empty());
    }
}
