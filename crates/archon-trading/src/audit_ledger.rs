use crate::maker_checker::MakerCheckerApproval;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const RETENTION_YEARS: u16 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    Requested,
    Accepted,
    Partial,
    Filled,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxFields {
    pub jurisdiction: String,
    pub account_type: String,
    pub tax_lot_method: String,
    pub wash_sale_relevant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub sequence: u64,
    pub prev_hash: String,
    pub content_hash: String,
    pub timestamp_unix_ms: u128,
    pub actor: String,
    pub strategy_id: String,
    pub policy_version: String,
    pub status: OrderStatus,
    pub risk_decision: Value,
    pub order_intent: Value,
    pub broker_response: Value,
    pub account: Value,
    pub tax: TaxFields,
    pub artefact_hashes: Vec<String>,
    pub maker_checker: Option<MakerCheckerApproval>,
    pub signature: String,
    pub retain_until_year: u16,
}

#[derive(Debug, Clone)]
pub struct AuditLedger {
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct NewLedgerRecord {
    pub actor: String,
    pub strategy_id: String,
    pub policy_version: String,
    pub status: OrderStatus,
    pub risk_decision: Value,
    pub order_intent: Value,
    pub broker_response: Value,
    pub account: Value,
    pub tax: TaxFields,
    pub artefacts: Vec<Vec<u8>>,
    pub maker_checker: Option<MakerCheckerApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditLedgerError {
    Io(String),
    Encode(String),
    ChainBroken,
    MakerChecker(String),
    LogAfterAct,
    RetentionViolation,
}

impl AuditLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AuditLedgerError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| AuditLedgerError::Io(err.to_string()))?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|file| file.sync_all())
            .map_err(|err| AuditLedgerError::Io(err.to_string()))?;
        Ok(Self { path })
    }

    pub fn log_before_act(
        &self,
        request: NewLedgerRecord,
    ) -> Result<LedgerRecord, AuditLedgerError> {
        if request.status != OrderStatus::Requested {
            return Err(AuditLedgerError::LogAfterAct);
        }
        self.append_record(request)
    }

    pub fn append_status(
        &self,
        request: NewLedgerRecord,
    ) -> Result<LedgerRecord, AuditLedgerError> {
        self.append_record(request)
    }

    pub fn verify_chain(&self) -> Result<(), AuditLedgerError> {
        let mut previous = ZERO_HASH.to_string();
        for record in self.records()? {
            if record.prev_hash != previous || record.content_hash != content_hash(&record)? {
                return Err(AuditLedgerError::ChainBroken);
            }
            previous = record.content_hash;
        }
        Ok(())
    }

    pub fn reconstruct_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<Vec<LedgerRecord>, AuditLedgerError> {
        self.verify_chain()?;
        Ok(self
            .records()?
            .into_iter()
            .filter(|record| record.strategy_id == strategy_id)
            .collect())
    }

    pub fn records(&self) -> Result<Vec<LedgerRecord>, AuditLedgerError> {
        let file = File::open(&self.path).map_err(|err| AuditLedgerError::Io(err.to_string()))?;
        BufReader::new(file)
            .lines()
            .filter(|line| line.as_ref().map_or(true, |text| !text.trim().is_empty()))
            .map(|line| {
                let line = line.map_err(|err| AuditLedgerError::Io(err.to_string()))?;
                serde_json::from_str(&line).map_err(|err| AuditLedgerError::Encode(err.to_string()))
            })
            .collect()
    }

    fn append_record(&self, request: NewLedgerRecord) -> Result<LedgerRecord, AuditLedgerError> {
        if let Some(approval) = &request.maker_checker {
            approval
                .verify_pair()
                .map_err(|err| AuditLedgerError::MakerChecker(err.to_string()))?;
        }
        let records = self.records()?;
        let previous = records
            .last()
            .map_or(ZERO_HASH.to_string(), |record| record.content_hash.clone());
        let mut record = request.into_record(records.len() as u64, previous);
        record.redact();
        record.content_hash = content_hash(&record)?;
        record.signature = sign_content(&record.content_hash, &record.actor);
        ensure_retention(&record)?;
        let encoded =
            serde_json::to_vec(&record).map_err(|err| AuditLedgerError::Encode(err.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|err| AuditLedgerError::Io(err.to_string()))?;
        file.write_all(&encoded)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_all())
            .map_err(|err| AuditLedgerError::Io(err.to_string()))?;
        Ok(record)
    }
}

impl NewLedgerRecord {
    fn into_record(self, sequence: u64, prev_hash: String) -> LedgerRecord {
        LedgerRecord {
            sequence,
            prev_hash,
            content_hash: String::new(),
            timestamp_unix_ms: now_ms(),
            actor: self.actor,
            strategy_id: self.strategy_id,
            policy_version: self.policy_version,
            status: self.status,
            risk_decision: self.risk_decision,
            order_intent: self.order_intent,
            broker_response: self.broker_response,
            account: self.account,
            tax: self.tax,
            artefact_hashes: self
                .artefacts
                .iter()
                .map(|bytes| blake3_hex(bytes))
                .collect(),
            maker_checker: self.maker_checker,
            signature: String::new(),
            retain_until_year: current_year() + RETENTION_YEARS,
        }
    }
}

impl LedgerRecord {
    fn redact(&mut self) {
        redact_value(&mut self.risk_decision);
        redact_value(&mut self.order_intent);
        redact_value(&mut self.broker_response);
        redact_value(&mut self.account);
        if is_secret_like(&self.actor) {
            self.actor = "[REDACTED]".to_string();
        }
    }
}

impl AuditLedgerError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "ERR-AUDIT-IO",
            Self::Encode(_) => "ERR-AUDIT-ENCODE",
            Self::ChainBroken => "ERR-AUDIT-CHAIN-BROKEN",
            Self::MakerChecker(_) => "ERR-AUDIT-MAKER-CHECKER",
            Self::LogAfterAct => "ERR-AUDIT-LOG-AFTER-ACT",
            Self::RetentionViolation => "ERR-AUDIT-RETENTION",
        }
    }
}

impl std::fmt::Display for AuditLedgerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AuditLedgerError {}

fn content_hash(record: &LedgerRecord) -> Result<String, AuditLedgerError> {
    let mut value =
        serde_json::to_value(record).map_err(|err| AuditLedgerError::Encode(err.to_string()))?;
    if let Value::Object(map) = &mut value {
        map.insert("content_hash".to_string(), Value::String(String::new()));
        map.insert("signature".to_string(), Value::String(String::new()));
    }
    let bytes =
        serde_json::to_vec(&value).map_err(|err| AuditLedgerError::Encode(err.to_string()))?;
    Ok(blake3_hex(&bytes))
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => redact_map(map),
        Value::Array(items) => items.iter_mut().for_each(redact_value),
        Value::String(text) if is_secret_like(text) => *text = "[REDACTED]".to_string(),
        _ => {}
    }
}

fn redact_map(map: &mut Map<String, Value>) {
    for (key, value) in map.iter_mut() {
        if is_secret_key(key) {
            *value = Value::String("[REDACTED]".to_string());
        } else {
            redact_value(value);
        }
    }
}

fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "api_key",
        "apikey",
        "credential",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn is_secret_like(text: &str) -> bool {
    text.starts_with("sk-") || text.starts_with("pk_") || text.contains("BEGIN PRIVATE KEY")
}

fn ensure_retention(record: &LedgerRecord) -> Result<(), AuditLedgerError> {
    if record.retain_until_year < current_year() + RETENTION_YEARS {
        return Err(AuditLedgerError::RetentionViolation);
    }
    Ok(())
}

fn sign_content(content_hash: &str, actor: &str) -> String {
    blake3_hex(format!("{content_hash}:{actor}:archon-trading-ledger").as_bytes())
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn current_year() -> u16 {
    1970 + (now_ms() / 31_557_600_000) as u16
}

#[cfg(test)]
mod tests {
    use super::{AuditLedger, AuditLedgerError, NewLedgerRecord, OrderStatus, TaxFields};
    use crate::maker_checker::MakerCheckerApproval;
    use serde_json::json;
    use serial_test::serial;

    #[test]
    #[serial]
    fn log_before_act_redacts_before_durable_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let ledger = AuditLedger::open(&path).unwrap();
        ledger
            .log_before_act(sample_request(OrderStatus::Requested))
            .unwrap();
        let persisted = std::fs::read_to_string(path).unwrap();
        assert!(persisted.contains("[REDACTED]"));
        assert!(!persisted.contains(&secret_fixture()));
        assert!(ledger.verify_chain().is_ok());
    }

    #[test]
    #[serial]
    fn maker_checker_blocks_same_actor() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = AuditLedger::open(dir.path().join("ledger.jsonl")).unwrap();
        let mut request = sample_request(OrderStatus::Requested);
        request.maker_checker = Some(MakerCheckerApproval::new(
            "r1", "alice", "alice", "deploy", true, "ok",
        ));
        let error = ledger.log_before_act(request).unwrap_err();
        assert_eq!(error.code(), "ERR-AUDIT-MAKER-CHECKER");
    }

    #[test]
    #[serial]
    fn chain_tamper_is_detected_and_reconstruction_filters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let ledger = AuditLedger::open(&path).unwrap();
        ledger
            .log_before_act(sample_request(OrderStatus::Requested))
            .unwrap();
        ledger
            .append_status(sample_request(OrderStatus::Filled))
            .unwrap();
        assert_eq!(ledger.reconstruct_strategy("strat-a").unwrap().len(), 2);
        let tampered = std::fs::read_to_string(&path)
            .unwrap()
            .replace("Filled", "Rejected");
        std::fs::write(&path, tampered).unwrap();
        assert_eq!(ledger.verify_chain(), Err(AuditLedgerError::ChainBroken));
    }

    #[test]
    fn log_after_act_invariant_rejects_non_requested_preflight() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = AuditLedger::open(dir.path().join("ledger.jsonl")).unwrap();
        let error = ledger
            .log_before_act(sample_request(OrderStatus::Accepted))
            .unwrap_err();
        assert_eq!(error, AuditLedgerError::LogAfterAct);
    }

    fn sample_request(status: OrderStatus) -> NewLedgerRecord {
        NewLedgerRecord {
            actor: "trader".to_string(),
            strategy_id: "strat-a".to_string(),
            policy_version: "policy-v1".to_string(),
            status,
            risk_decision: json!({"allowed": true}),
            order_intent: json!({"symbol": "AAPL", "api_key": secret_fixture()}),
            broker_response: json!({"state": "none"}),
            account: json!({"account_id": "acct-1", "token": secret_fixture()}),
            tax: TaxFields {
                jurisdiction: "US".to_string(),
                account_type: "taxable".to_string(),
                tax_lot_method: "FIFO".to_string(),
                wash_sale_relevant: true,
            },
            artefacts: vec![b"compiled-script".to_vec()],
            maker_checker: Some(MakerCheckerApproval::new(
                "r1", "alice", "bob", "deploy", true, "ok",
            )),
        }
    }

    fn secret_fixture() -> String {
        ["sk", "live", "fixture"].join("-")
    }
}
