use crate::audit_ledger::{AuditLedger, NewLedgerRecord, OrderStatus, TaxFields};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TradingKbTopic {
    MarketStructure,
    ElliottWave,
    Macro,
    Crypto,
    Equities,
    RiskManagement,
    Execution,
    Backtesting,
    PineScript,
    StrategyPostmortems,
}

impl TradingKbTopic {
    pub const ALL: [Self; 10] = [
        Self::MarketStructure,
        Self::ElliottWave,
        Self::Macro,
        Self::Crypto,
        Self::Equities,
        Self::RiskManagement,
        Self::Execution,
        Self::Backtesting,
        Self::PineScript,
        Self::StrategyPostmortems,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::MarketStructure => "market-structure",
            Self::ElliottWave => "elliott-wave",
            Self::Macro => "macro",
            Self::Crypto => "crypto",
            Self::Equities => "equities",
            Self::RiskManagement => "risk-management",
            Self::Execution => "execution",
            Self::Backtesting => "backtesting",
            Self::PineScript => "pine-script",
            Self::StrategyPostmortems => "strategy-postmortems",
        }
    }
}

impl FromStr for TradingKbTopic {
    type Err = KbError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|topic| topic.key() == value)
            .ok_or(KbError::UnknownTopic)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Docs,
    Images,
    Videos,
    Transcripts,
    Books,
    Screenshots,
    BrokerDocs,
    PineExamples,
    Research,
}

impl MediaKind {
    pub const fn citable_after_extraction(self) -> bool {
        matches!(
            self,
            Self::Docs
                | Self::Transcripts
                | Self::Books
                | Self::BrokerDocs
                | Self::PineExamples
                | Self::Research
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractionState {
    Clean,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimStatus {
    Verified,
    UnverifiedHypothesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContradictionStatus {
    Contradiction,
    PossibleContradiction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimPolarity {
    Supports,
    Refutes,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceChunk {
    pub chunk_id: String,
    pub topic: TradingKbTopic,
    pub media_kind: MediaKind,
    pub content_hash: String,
    pub extraction_state: ExtractionState,
    pub source_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub chunk_id: String,
    pub chunk_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeClaim {
    pub claim_id: String,
    pub topic: TradingKbTopic,
    pub subject: String,
    pub statement: String,
    pub polarity: ClaimPolarity,
    pub citation: Citation,
    pub status: ClaimStatus,
    pub referenced_by_non_retired_strategy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contradiction {
    pub left_claim_id: String,
    pub right_claim_id: String,
    pub topic: TradingKbTopic,
    pub status: ContradictionStatus,
    pub escalation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewPacket {
    pub packet_id: String,
    pub contradictions: Vec<Contradiction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KbError {
    UnknownTopic,
    UnsupportedMedia,
    ExtractionDegradedNotCitable,
    CitationMissing,
    CitationHashMismatch,
    Audit(String),
}

#[derive(Debug)]
pub struct TradingKnowledgeBase {
    chunks: BTreeMap<String, SourceChunk>,
    claims: BTreeMap<String, KnowledgeClaim>,
    review_packets: Vec<ReviewPacket>,
    audit_ledger: Option<AuditLedger>,
}

impl TradingKnowledgeBase {
    pub fn new() -> Self {
        Self {
            chunks: BTreeMap::new(),
            claims: BTreeMap::new(),
            review_packets: Vec::new(),
            audit_ledger: None,
        }
    }

    pub fn with_audit_ledger(audit_ledger: AuditLedger) -> Self {
        Self {
            audit_ledger: Some(audit_ledger),
            ..Self::new()
        }
    }

    pub fn topics() -> [TradingKbTopic; 10] {
        TradingKbTopic::ALL
    }

    pub fn ingest_chunk(
        &mut self,
        chunk_id: impl Into<String>,
        topic: TradingKbTopic,
        media_kind: MediaKind,
        bytes: &[u8],
        source_uri: impl Into<String>,
        extraction_state: ExtractionState,
    ) -> Result<SourceChunk, KbError> {
        if !media_kind.citable_after_extraction() && extraction_state == ExtractionState::Degraded {
            return Err(KbError::ExtractionDegradedNotCitable);
        }
        let chunk = SourceChunk {
            chunk_id: chunk_id.into(),
            topic,
            media_kind,
            content_hash: hash_bytes(bytes),
            extraction_state,
            source_uri: source_uri.into(),
        };
        self.chunks.insert(chunk.chunk_id.clone(), chunk.clone());
        Ok(chunk)
    }

    pub fn add_claim(&mut self, mut claim: KnowledgeClaim) -> Result<KnowledgeClaim, KbError> {
        self.validate_citation(&claim)?;
        claim.status = ClaimStatus::Verified;
        self.persist_provenance(&claim)?;
        self.claims.insert(claim.claim_id.clone(), claim.clone());
        Ok(claim)
    }

    pub fn verify_claim_integrity(&mut self) -> Vec<String> {
        let mut reverted = Vec::new();
        let chunks = &self.chunks;
        for claim in self.claims.values_mut() {
            if !citation_matches(chunks, claim) {
                claim.status = ClaimStatus::UnverifiedHypothesis;
                reverted.push(claim.claim_id.clone());
            }
        }
        reverted
    }

    pub fn detect_contradictions(&mut self) -> Option<ReviewPacket> {
        let verified: Vec<_> = self
            .claims
            .values()
            .filter(|claim| claim.status == ClaimStatus::Verified)
            .cloned()
            .collect();
        let contradictions = find_contradictions(&verified);
        if contradictions.is_empty() {
            return None;
        }
        let packet = ReviewPacket {
            packet_id: format!("review-{}", self.review_packets.len() + 1),
            contradictions,
        };
        self.review_packets.push(packet.clone());
        Some(packet)
    }

    pub fn gc_candidates(&self) -> Vec<String> {
        let pinned = self.non_retired_referenced_chunks();
        self.chunks
            .keys()
            .filter(|chunk_id| !pinned.contains(*chunk_id))
            .cloned()
            .collect()
    }

    pub fn non_retired_referenced_chunks(&self) -> BTreeSet<String> {
        self.claims
            .values()
            .filter(|claim| claim.referenced_by_non_retired_strategy)
            .map(|claim| claim.citation.chunk_id.clone())
            .collect()
    }

    pub fn review_packets(&self) -> &[ReviewPacket] {
        &self.review_packets
    }

    pub fn claim_status(&self, claim_id: &str) -> Option<ClaimStatus> {
        self.claims.get(claim_id).map(|claim| claim.status)
    }

    fn validate_citation(&self, claim: &KnowledgeClaim) -> Result<(), KbError> {
        let chunk = self
            .chunks
            .get(&claim.citation.chunk_id)
            .ok_or(KbError::CitationMissing)?;
        if !chunk.media_kind.citable_after_extraction() {
            return Err(KbError::UnsupportedMedia);
        }
        if chunk.extraction_state == ExtractionState::Degraded {
            return Err(KbError::ExtractionDegradedNotCitable);
        }
        if chunk.content_hash != claim.citation.chunk_hash || chunk.topic != claim.topic {
            return Err(KbError::CitationHashMismatch);
        }
        Ok(())
    }

    fn persist_provenance(&self, claim: &KnowledgeClaim) -> Result<(), KbError> {
        let Some(ledger) = &self.audit_ledger else {
            return Ok(());
        };
        let record = NewLedgerRecord {
            actor: "trading-kb".to_string(),
            strategy_id: claim.claim_id.clone(),
            policy_version: "kb-provenance-v1".to_string(),
            status: OrderStatus::Requested,
            risk_decision: json!({"kb_topic": claim.topic.key(), "status": "provenance"}),
            order_intent: json!({"claim": claim.statement, "citation": claim.citation}),
            broker_response: json!({"provider_neutral": true}),
            account: json!({"system": "archon-knowledge"}),
            tax: TaxFields {
                jurisdiction: "N/A".to_string(),
                account_type: "N/A".to_string(),
                tax_lot_method: "N/A".to_string(),
                wash_sale_relevant: false,
            },
            artefacts: vec![claim.statement.as_bytes().to_vec()],
            maker_checker: None,
        };
        ledger
            .log_before_act(record)
            .map_err(|error| KbError::Audit(error.to_string()))?;
        Ok(())
    }
}

impl Default for TradingKnowledgeBase {
    fn default() -> Self {
        Self::new()
    }
}

impl KbError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownTopic => "ERR-KB-UNKNOWN-TOPIC",
            Self::UnsupportedMedia => "ERR-KB-UNSUPPORTED-MEDIA",
            Self::ExtractionDegradedNotCitable => "ERR-KB-EXTRACTION-DEGRADED",
            Self::CitationMissing => "ERR-KB-CITATION-MISSING",
            Self::CitationHashMismatch => "ERR-KB-CITATION-HASH-MISMATCH",
            Self::Audit(_) => "ERR-KB-AUDIT",
        }
    }
}

impl std::fmt::Display for KbError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for KbError {}

fn citation_matches(chunks: &BTreeMap<String, SourceChunk>, claim: &KnowledgeClaim) -> bool {
    chunks
        .get(&claim.citation.chunk_id)
        .is_some_and(|chunk| chunk.content_hash == claim.citation.chunk_hash)
}

fn find_contradictions(claims: &[KnowledgeClaim]) -> Vec<Contradiction> {
    let mut contradictions = Vec::new();
    for left_index in 0..claims.len() {
        for right in claims.iter().skip(left_index + 1) {
            if let Some(contradiction) = contradiction_between(&claims[left_index], right) {
                contradictions.push(contradiction);
            }
        }
    }
    contradictions
}

fn contradiction_between(left: &KnowledgeClaim, right: &KnowledgeClaim) -> Option<Contradiction> {
    if left.topic != right.topic || normalize(&left.subject) != normalize(&right.subject) {
        return None;
    }
    let status = match (left.polarity, right.polarity) {
        (ClaimPolarity::Supports, ClaimPolarity::Refutes)
        | (ClaimPolarity::Refutes, ClaimPolarity::Supports) => ContradictionStatus::Contradiction,
        (ClaimPolarity::Uncertain, _) | (_, ClaimPolarity::Uncertain) => {
            ContradictionStatus::PossibleContradiction
        }
        _ => return None,
    };
    Some(Contradiction {
        left_claim_id: left.claim_id.clone(),
        right_claim_id: right.claim_id.clone(),
        topic: left.topic,
        status,
        escalation: "PER-01".to_string(),
    })
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
