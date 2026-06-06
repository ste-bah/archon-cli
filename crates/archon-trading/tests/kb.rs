use archon_trading::audit_ledger::AuditLedger;
use archon_trading::kb::{
    Citation, ClaimPolarity, ClaimStatus, ContradictionStatus, ExtractionState, KbError,
    KnowledgeClaim, MediaKind, SourceChunk, TradingKbTopic, TradingKnowledgeBase,
};

#[test]
fn exposes_closed_ten_topic_taxonomy_and_rejects_unknown_key() {
    assert_eq!(TradingKnowledgeBase::topics().len(), 10);
    assert_eq!(
        "elliott-wave".parse::<TradingKbTopic>().unwrap(),
        TradingKbTopic::ElliottWave
    );
    assert_eq!(
        "unknown".parse::<TradingKbTopic>().unwrap_err(),
        KbError::UnknownTopic
    );
}

#[test]
fn citation_hash_change_reverts_claim_to_unverified_hypothesis() {
    let mut kb = TradingKnowledgeBase::new();
    let chunk = kb
        .ingest_chunk(
            "chunk-1",
            TradingKbTopic::Backtesting,
            MediaKind::Research,
            b"walk-forward evidence",
            "research://wf",
            ExtractionState::Clean,
        )
        .unwrap();
    kb.add_claim(sample_claim("c1", chunk)).unwrap();
    kb.ingest_chunk(
        "chunk-1",
        TradingKbTopic::Backtesting,
        MediaKind::Research,
        b"changed evidence",
        "research://wf",
        ExtractionState::Clean,
    )
    .unwrap();
    assert_eq!(kb.verify_claim_integrity(), vec!["c1".to_string()]);
    assert_eq!(
        kb.claim_status("c1"),
        Some(ClaimStatus::UnverifiedHypothesis)
    );
}

#[test]
fn contradiction_packet_escalates_to_per_01() {
    let mut kb = TradingKnowledgeBase::new();
    let left = chunk(&mut kb, "left", b"rates bullish");
    let right = chunk(&mut kb, "right", b"rates bearish");
    kb.add_claim(sample_claim_with_polarity(
        "c1",
        left,
        "rates",
        ClaimPolarity::Supports,
    ))
    .unwrap();
    kb.add_claim(sample_claim_with_polarity(
        "c2",
        right,
        "rates",
        ClaimPolarity::Refutes,
    ))
    .unwrap();
    let packet = kb.detect_contradictions().unwrap();
    assert_eq!(
        packet.contradictions[0].status,
        ContradictionStatus::Contradiction
    );
    assert_eq!(packet.contradictions[0].escalation, "PER-01");
}

#[test]
fn degraded_or_unsupported_media_is_not_citable() {
    let mut kb = TradingKnowledgeBase::new();
    let degraded = kb.ingest_chunk(
        "img",
        TradingKbTopic::Execution,
        MediaKind::Images,
        b"ocr fail",
        "img://1",
        ExtractionState::Degraded,
    );
    assert_eq!(degraded.unwrap_err(), KbError::ExtractionDegradedNotCitable);
    let chunk = kb
        .ingest_chunk(
            "img2",
            TradingKbTopic::Execution,
            MediaKind::Images,
            b"chart",
            "img://2",
            ExtractionState::Clean,
        )
        .unwrap();
    assert_eq!(
        kb.add_claim(sample_claim("c1", chunk)).unwrap_err(),
        KbError::UnsupportedMedia
    );
}

#[test]
fn non_retired_strategy_references_are_not_gc_candidates() {
    let mut kb = TradingKnowledgeBase::new();
    let pinned = pine_chunk(&mut kb, "pinned", "pine://1");
    let free = pine_chunk(&mut kb, "free", "pine://2");
    let mut claim = sample_claim("pinned-claim", pinned);
    claim.referenced_by_non_retired_strategy = true;
    kb.add_claim(claim).unwrap();
    kb.add_claim(sample_claim("free-claim", free)).unwrap();
    assert_eq!(kb.gc_candidates(), vec!["free".to_string()]);
}

#[test]
fn provenance_can_be_persisted_to_audit_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = AuditLedger::open(dir.path().join("kb-ledger.jsonl")).unwrap();
    let mut kb = TradingKnowledgeBase::with_audit_ledger(ledger.clone());
    let chunk = kb
        .ingest_chunk(
            "chunk",
            TradingKbTopic::Equities,
            MediaKind::Docs,
            b"10-K",
            "sec://aapl",
            ExtractionState::Clean,
        )
        .unwrap();
    kb.add_claim(sample_claim("claim", chunk)).unwrap();
    assert_eq!(ledger.reconstruct_strategy("claim").unwrap().len(), 1);
}

fn chunk(kb: &mut TradingKnowledgeBase, id: &str, bytes: &[u8]) -> SourceChunk {
    kb.ingest_chunk(
        id,
        TradingKbTopic::Macro,
        MediaKind::Docs,
        bytes,
        format!("doc://{id}"),
        ExtractionState::Clean,
    )
    .unwrap()
}

fn pine_chunk(kb: &mut TradingKnowledgeBase, id: &str, uri: &str) -> SourceChunk {
    kb.ingest_chunk(
        id,
        TradingKbTopic::PineScript,
        MediaKind::PineExamples,
        b"//@version=6",
        uri,
        ExtractionState::Clean,
    )
    .unwrap()
}

fn sample_claim(claim_id: &str, chunk: SourceChunk) -> KnowledgeClaim {
    sample_claim_with_polarity(claim_id, chunk, "subject", ClaimPolarity::Supports)
}

fn sample_claim_with_polarity(
    claim_id: &str,
    chunk: SourceChunk,
    subject: &str,
    polarity: ClaimPolarity,
) -> KnowledgeClaim {
    KnowledgeClaim {
        claim_id: claim_id.to_string(),
        topic: chunk.topic,
        subject: subject.to_string(),
        statement: "claim text".to_string(),
        polarity,
        citation: Citation {
            chunk_id: chunk.chunk_id,
            chunk_hash: chunk.content_hash,
        },
        status: ClaimStatus::UnverifiedHypothesis,
        referenced_by_non_retired_strategy: false,
    }
}
