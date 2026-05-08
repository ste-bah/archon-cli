use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::runtime_models::ProviderRuntimeEventRecord;

pub fn insert_provider_runtime_event(
    db: &DbInstance,
    event: &ProviderRuntimeEventRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event.event_id.as_str()));
    params.insert("pid".into(), DataValue::from(event.provider_id.as_str()));
    params.insert(
        "prof".into(),
        DataValue::from(event.profile_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "model".into(),
        DataValue::from(event.model_id.as_deref().unwrap_or("")),
    );
    params.insert("mode".into(), DataValue::from(event.runtime_mode.as_str()));
    params.insert("etype".into(), DataValue::from(event.event_type.as_str()));
    params.insert("sev".into(), DataValue::from(event.severity.as_str()));
    params.insert(
        "reason".into(),
        DataValue::from(event.reason_code.as_deref().unwrap_or("")),
    );
    params.insert(
        "msg".into(),
        DataValue::from(event.message.as_deref().unwrap_or("")),
    );
    params.insert(
        "retry".into(),
        DataValue::from(event.retry_count.unwrap_or(0) as i64),
    );
    params.insert(
        "ffrom".into(),
        DataValue::from(event.fallback_from.as_deref().unwrap_or("")),
    );
    params.insert(
        "fto".into(),
        DataValue::from(event.fallback_to.as_deref().unwrap_or("")),
    );
    params.insert(
        "req".into(),
        DataValue::from(event.request_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "run".into(),
        DataValue::from(event.run_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "pipe".into(),
        DataValue::from(event.pipeline_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "raw".into(),
        DataValue::from(event.raw_redacted_json.to_string().as_str()),
    );
    params.insert("created".into(), DataValue::from(event.created_at.as_str()));

    db.run_script(
        "?[event_id, provider_id, profile_id, model_id, runtime_mode, \
         event_type, severity, reason_code, message, retry_count, \
         fallback_from, fallback_to, request_id, run_id, pipeline_id, \
         raw_redacted_json, created_at] <- [[$eid, $pid, $prof, $model, \
         $mode, $etype, $sev, $reason, $msg, $retry, $ffrom, $fto, $req, \
         $run, $pipe, $raw, $created]] \
         :put provider_runtime_events { event_id => provider_id, profile_id, \
         model_id, runtime_mode, event_type, severity, reason_code, message, \
         retry_count, fallback_from, fallback_to, request_id, run_id, \
         pipeline_id, raw_redacted_json, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert provider_runtime_events failed: {e}"))?;

    Ok(())
}

pub fn get_provider_runtime_event(
    db: &DbInstance,
    event_id: &str,
) -> Result<Option<ProviderRuntimeEventRecord>> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event_id));

    let result = db
        .run_script(
            provider_runtime_event_query("event_id = $eid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get provider_runtime_event failed: {e}"))?;

    Ok(result.rows.first().map(|row| row_to_provider_event(row)))
}

pub fn list_provider_runtime_events(
    db: &DbInstance,
    provider_id: Option<&str>,
) -> Result<Vec<ProviderRuntimeEventRecord>> {
    let result = if let Some(provider_id) = provider_id {
        let mut params = BTreeMap::new();
        params.insert("pid".into(), DataValue::from(provider_id));
        db.run_script(
            provider_runtime_event_query("provider_id = $pid"),
            params,
            ScriptMutability::Immutable,
        )
    } else {
        db.run_script(
            provider_runtime_event_query("true"),
            Default::default(),
            ScriptMutability::Immutable,
        )
    }
    .map_err(|e| anyhow::anyhow!("list provider_runtime_events failed: {e}"))?;

    let mut events: Vec<_> = result
        .rows
        .iter()
        .map(|row| row_to_provider_event(row))
        .collect();
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(events)
}

fn provider_runtime_event_query(predicate: &'static str) -> &'static str {
    match predicate {
        "event_id = $eid" => {
            "?[event_id, provider_id, profile_id, model_id, runtime_mode, \
             event_type, severity, reason_code, message, retry_count, \
             fallback_from, fallback_to, request_id, run_id, pipeline_id, \
             raw_redacted_json, created_at] := *provider_runtime_events{ \
             event_id, provider_id, profile_id, model_id, runtime_mode, \
             event_type, severity, reason_code, message, retry_count, \
             fallback_from, fallback_to, request_id, run_id, pipeline_id, \
             raw_redacted_json, created_at }, event_id = $eid"
        }
        "provider_id = $pid" => {
            "?[event_id, provider_id, profile_id, model_id, runtime_mode, \
             event_type, severity, reason_code, message, retry_count, \
             fallback_from, fallback_to, request_id, run_id, pipeline_id, \
             raw_redacted_json, created_at] := *provider_runtime_events{ \
             event_id, provider_id, profile_id, model_id, runtime_mode, \
             event_type, severity, reason_code, message, retry_count, \
             fallback_from, fallback_to, request_id, run_id, pipeline_id, \
             raw_redacted_json, created_at }, provider_id = $pid"
        }
        _ => {
            "?[event_id, provider_id, profile_id, model_id, runtime_mode, \
             event_type, severity, reason_code, message, retry_count, \
             fallback_from, fallback_to, request_id, run_id, pipeline_id, \
             raw_redacted_json, created_at] := *provider_runtime_events{ \
             event_id, provider_id, profile_id, model_id, runtime_mode, \
             event_type, severity, reason_code, message, retry_count, \
             fallback_from, fallback_to, request_id, run_id, pipeline_id, \
             raw_redacted_json, created_at }"
        }
    }
}

fn row_to_provider_event(row: &[DataValue]) -> ProviderRuntimeEventRecord {
    ProviderRuntimeEventRecord {
        event_id: row[0].get_str().unwrap_or("").to_string(),
        provider_id: row[1].get_str().unwrap_or("").to_string(),
        profile_id: non_empty(row[2].get_str().unwrap_or("")),
        model_id: non_empty(row[3].get_str().unwrap_or("")),
        runtime_mode: row[4].get_str().unwrap_or("").to_string(),
        event_type: row[5].get_str().unwrap_or("").to_string(),
        severity: row[6].get_str().unwrap_or("").to_string(),
        reason_code: non_empty(row[7].get_str().unwrap_or("")),
        message: non_empty(row[8].get_str().unwrap_or("")),
        retry_count: {
            let count = row[9].get_int().unwrap_or(0);
            if count > 0 { Some(count as u32) } else { None }
        },
        fallback_from: non_empty(row[10].get_str().unwrap_or("")),
        fallback_to: non_empty(row[11].get_str().unwrap_or("")),
        request_id: non_empty(row[12].get_str().unwrap_or("")),
        run_id: non_empty(row[13].get_str().unwrap_or("")),
        pipeline_id: non_empty(row[14].get_str().unwrap_or("")),
        raw_redacted_json: serde_json::from_str(row[15].get_str().unwrap_or("{}"))
            .unwrap_or_else(|_| serde_json::json!({})),
        created_at: row[16].get_str().unwrap_or("").to_string(),
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-provider-runtime-events-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn provider_runtime_event_roundtrips_through_cozo() {
        let db = test_db();
        let event = ProviderRuntimeEventRecord::new(
            "provider-event-1",
            "anthropic",
            "direct",
            "spoof_identity_selected",
            "info",
            "2026-05-08T12:00:00Z",
        )
        .with_profile("oauth-main")
        .with_model("claude-sonnet-4-6")
        .with_reason("oauth")
        .with_redacted_json(serde_json::json!({"spoof_reason": "oauth"}));

        insert_provider_runtime_event(&db, &event).unwrap();
        let restored = get_provider_runtime_event(&db, "provider-event-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.provider_id, "anthropic");
        assert_eq!(restored.profile_id.as_deref(), Some("oauth-main"));
        assert_eq!(restored.raw_redacted_json["spoof_reason"], "oauth");
    }

    #[test]
    fn provider_runtime_events_filter_by_provider() {
        let db = test_db();
        insert_provider_runtime_event(
            &db,
            &ProviderRuntimeEventRecord::new(
                "provider-event-1",
                "anthropic",
                "direct",
                "request_started",
                "debug",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_provider_runtime_event(
            &db,
            &ProviderRuntimeEventRecord::new(
                "provider-event-2",
                "openai",
                "direct",
                "request_succeeded",
                "info",
                "2026-05-08T12:01:00Z",
            ),
        )
        .unwrap();

        let all = list_provider_runtime_events(&db, None).unwrap();
        let anthropic = list_provider_runtime_events(&db, Some("anthropic")).unwrap();

        assert_eq!(all.len(), 2);
        assert_eq!(all[0].event_id, "provider-event-2");
        assert_eq!(anthropic.len(), 1);
        assert_eq!(anthropic[0].provider_id, "anthropic");
    }
}
