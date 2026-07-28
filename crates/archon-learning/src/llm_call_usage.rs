use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::cozo_guard::run_script_guarded;
use crate::errors::COZO_RELATION_ALREADY_EXISTS;

pub(crate) fn ensure_schema(db: &DbInstance) -> Result<()> {
    let script = r#":create llm_call_usage {
        request_id: String => run_id: String default "", session_id: String default "",
        turn: Int default -1, round: Int default -1, role: String default "",
        origin: String default "", provider_id: String, model_id: String,
        input_available: Bool, input_tokens: Int, output_available: Bool,
        output_tokens: Int, cache_creation_available: Bool, cache_creation_tokens: Int,
        cache_read_available: Bool, cache_read_tokens: Int, context_tokens: Int default -1,
        effective_denominator: Int default -1, terminal_status: String, created_at: String,
    }"#;
    match run_script_guarded(
        db,
        script,
        Default::default(),
        ScriptMutability::Mutable,
        "create llm_call_usage failed",
    ) {
        Ok(_) => Ok(()),
        Err(error)
            if COZO_RELATION_ALREADY_EXISTS
                .iter()
                .any(|marker| error.to_string().contains(marker)) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageAvailability {
    Unavailable,
    Known(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCallUsageRecord {
    pub request_id: String,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub turn: Option<u64>,
    pub round: Option<u64>,
    pub role: Option<String>,
    pub origin: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    pub input_tokens: UsageAvailability,
    pub output_tokens: UsageAvailability,
    pub cache_creation_input_tokens: UsageAvailability,
    pub cache_read_input_tokens: UsageAvailability,
    pub context_input_tokens: Option<u64>,
    pub effective_denominator: Option<u64>,
    pub terminal_status: String,
    pub created_at: String,
}

impl LlmCallUsageRecord {
    pub fn effective_burn(&self) -> Option<u64> {
        let input = known(&self.input_tokens)?;
        let output = known(&self.output_tokens)?;
        input.checked_add(output)
    }
}

#[derive(Clone, Debug, Default)]
pub struct LlmCallUsageScope {
    pub run_id: Option<String>,
    pub session_id: Option<String>,
}

impl LlmCallUsageScope {
    pub fn new(run_id: Option<&str>, session_id: Option<&str>) -> Self {
        Self {
            run_id: run_id.map(ToOwned::to_owned),
            session_id: session_id.map(ToOwned::to_owned),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertLlmCallUsageOutcome {
    Created,
    Reused,
    Conflict,
}

pub fn insert_llm_call_usage(
    db: &DbInstance,
    record: &LlmCallUsageRecord,
) -> Result<InsertLlmCallUsageOutcome> {
    archon_cozo::run_bound_guarded(
        db,
        "insert llm_call_usage failed",
        ScriptMutability::Mutable,
        || classify_write(db, record),
    )
}

fn classify_write(
    db: &DbInstance,
    record: &LlmCallUsageRecord,
) -> Result<InsertLlmCallUsageOutcome> {
    match db.run_script(
        insert_script(),
        insert_params(record)?,
        ScriptMutability::Mutable,
    ) {
        Ok(_) => Ok(InsertLlmCallUsageOutcome::Created),
        Err(_) => match get_llm_call_usage(db, &record.request_id)? {
            Some(existing) if existing == *record => Ok(InsertLlmCallUsageOutcome::Reused),
            Some(_) => Ok(InsertLlmCallUsageOutcome::Conflict),
            None => Err(anyhow::anyhow!(
                "insert llm_call_usage failed without a durable row"
            )),
        },
    }
}

pub fn get_llm_call_usage(db: &DbInstance, request_id: &str) -> Result<Option<LlmCallUsageRecord>> {
    let mut params = BTreeMap::new();
    params.insert("request_id".into(), DataValue::from(request_id));
    let rows = run_script_guarded(
        db,
        query_by_request_id(),
        params,
        ScriptMutability::Immutable,
        "get llm_call_usage failed",
    )?;
    Ok(rows.rows.first().map(|row| row_to_record(row)))
}

pub fn list_llm_call_usage(
    db: &DbInstance,
    scope: &LlmCallUsageScope,
) -> Result<Vec<LlmCallUsageRecord>> {
    let mut params = BTreeMap::new();
    params.insert("run".into(), string_value(scope.run_id.as_deref()));
    params.insert("session".into(), string_value(scope.session_id.as_deref()));
    let rows = run_script_guarded(
        db,
        query_by_scope(),
        params,
        ScriptMutability::Immutable,
        "list llm_call_usage failed",
    )?;
    Ok(rows.rows.iter().map(|row| row_to_record(row)).collect())
}

fn known(value: &UsageAvailability) -> Option<u64> {
    match value {
        UsageAvailability::Unavailable => None,
        UsageAvailability::Known(value) => Some(*value),
    }
}

fn checked_int(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| anyhow::anyhow!("{field} exceeds Cozo Int range"))
}

fn optional_int(value: Option<u64>, field: &str) -> Result<i64> {
    value.map_or(Ok(-1), |value| checked_int(value, field))
}

fn usage_params(
    params: &mut BTreeMap<String, DataValue>,
    name: &str,
    value: &UsageAvailability,
) -> Result<()> {
    let (available, value) = match value {
        UsageAvailability::Unavailable => (false, 0),
        UsageAvailability::Known(value) => (true, checked_int(*value, name)?),
    };
    params.insert(format!("{name}_available"), DataValue::from(available));
    params.insert(name.into(), DataValue::from(value));
    Ok(())
}

fn insert_params(record: &LlmCallUsageRecord) -> Result<BTreeMap<String, DataValue>> {
    let mut params = BTreeMap::new();
    params.insert(
        "request_id".into(),
        DataValue::from(record.request_id.as_str()),
    );
    params.insert("run".into(), string_value(record.run_id.as_deref()));
    params.insert("session".into(), string_value(record.session_id.as_deref()));
    params.insert("turn".into(), optional_int(record.turn, "turn")?.into());
    params.insert("round".into(), optional_int(record.round, "round")?.into());
    params.insert("role".into(), string_value(record.role.as_deref()));
    params.insert("origin".into(), string_value(record.origin.as_deref()));
    params.insert(
        "provider".into(),
        DataValue::from(record.provider_id.as_str()),
    );
    params.insert("model".into(), DataValue::from(record.model_id.as_str()));
    usage_params(&mut params, "input_tokens", &record.input_tokens)?;
    usage_params(&mut params, "output_tokens", &record.output_tokens)?;
    usage_params(
        &mut params,
        "cache_creation",
        &record.cache_creation_input_tokens,
    )?;
    usage_params(&mut params, "cache_read", &record.cache_read_input_tokens)?;
    params.insert(
        "context".into(),
        optional_int(record.context_input_tokens, "context")?.into(),
    );
    params.insert(
        "denominator".into(),
        optional_int(record.effective_denominator, "denominator")?.into(),
    );
    params.insert(
        "status".into(),
        DataValue::from(record.terminal_status.as_str()),
    );
    params.insert(
        "created".into(),
        DataValue::from(record.created_at.as_str()),
    );
    Ok(params)
}

fn string_value(value: Option<&str>) -> DataValue {
    DataValue::from(value.unwrap_or(""))
}

fn insert_script() -> &'static str {
    "?[request_id, run_id, session_id, turn, round, role, origin, provider_id, model_id, input_available, input_tokens, output_available, output_tokens, cache_creation_available, cache_creation_tokens, cache_read_available, cache_read_tokens, context_tokens, effective_denominator, terminal_status, created_at] <- [[$request_id, $run, $session, $turn, $round, $role, $origin, $provider, $model, $input_tokens_available, $input_tokens, $output_tokens_available, $output_tokens, $cache_creation_available, $cache_creation, $cache_read_available, $cache_read, $context, $denominator, $status, $created]] :insert llm_call_usage { request_id => run_id, session_id, turn, round, role, origin, provider_id, model_id, input_available, input_tokens, output_available, output_tokens, cache_creation_available, cache_creation_tokens, cache_read_available, cache_read_tokens, context_tokens, effective_denominator, terminal_status, created_at }"
}

fn query_by_request_id() -> &'static str {
    "?[request_id, run_id, session_id, turn, round, role, origin, provider_id, model_id, input_available, input_tokens, output_available, output_tokens, cache_creation_available, cache_creation_tokens, cache_read_available, cache_read_tokens, context_tokens, effective_denominator, terminal_status, created_at] := *llm_call_usage { request_id, run_id, session_id, turn, round, role, origin, provider_id, model_id, input_available, input_tokens, output_available, output_tokens, cache_creation_available, cache_creation_tokens, cache_read_available, cache_read_tokens, context_tokens, effective_denominator, terminal_status, created_at }, request_id = $request_id"
}

fn query_by_scope() -> &'static str {
    "?[request_id, run_id, session_id, turn, round, role, origin, provider_id, model_id, input_available, input_tokens, output_available, output_tokens, cache_creation_available, cache_creation_tokens, cache_read_available, cache_read_tokens, context_tokens, effective_denominator, terminal_status, created_at] := *llm_call_usage { request_id, run_id, session_id, turn, round, role, origin, provider_id, model_id, input_available, input_tokens, output_available, output_tokens, cache_creation_available, cache_creation_tokens, cache_read_available, cache_read_tokens, context_tokens, effective_denominator, terminal_status, created_at }, run_id = $run, session_id = $session"
}

fn row_to_record(row: &[DataValue]) -> LlmCallUsageRecord {
    LlmCallUsageRecord {
        request_id: text(row, 0).into(),
        run_id: non_empty(text(row, 1)),
        session_id: non_empty(text(row, 2)),
        turn: optional_u64(row, 3),
        round: optional_u64(row, 4),
        role: non_empty(text(row, 5)),
        origin: non_empty(text(row, 6)),
        provider_id: text(row, 7).into(),
        model_id: text(row, 8).into(),
        input_tokens: usage_from(row, 9, 10),
        output_tokens: usage_from(row, 11, 12),
        cache_creation_input_tokens: usage_from(row, 13, 14),
        cache_read_input_tokens: usage_from(row, 15, 16),
        context_input_tokens: optional_u64(row, 17),
        effective_denominator: optional_u64(row, 18),
        terminal_status: text(row, 19).into(),
        created_at: text(row, 20).into(),
    }
}

fn text(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}
fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.into())
}
fn optional_u64(row: &[DataValue], index: usize) -> Option<u64> {
    u64::try_from(row[index].get_int().unwrap_or(-1)).ok()
}
fn usage_from(row: &[DataValue], available: usize, value: usize) -> UsageAvailability {
    if row[available].get_bool().unwrap_or(false) {
        UsageAvailability::Known(row[value].get_int().unwrap_or(0) as u64)
    } else {
        UsageAvailability::Unavailable
    }
}
