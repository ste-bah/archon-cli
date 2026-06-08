use super::*;
use crate::command::registry::default_registry;
use crate::command::test_support::{CtxBuilder, drain_tui_events};
use archon_docs::models::{ChunkArtifact, DocumentStatus, SourceDocument};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn default_registry_registers_evidence_view_primaries() {
    let registry = default_registry();
    assert!(registry.is_primary("docs"));
    assert!(registry.is_primary("learning"));
}

#[test]
fn docs_usage_lists_prd_command_family() {
    let (mut ctx, mut rx) = CtxBuilder::new().build();
    DocsViewHandler
        .execute(&mut ctx, &[String::from("help")])
        .unwrap();
    let events = drain_tui_events(&mut rx);
    let text = match &events[0] {
        TuiEvent::TextDelta(text) => text,
        other => panic!("expected TextDelta, got {other:?}"),
    };
    for subcommand in DOCS_SUBCOMMANDS {
        assert!(text.contains(subcommand), "missing {subcommand}");
    }
}

#[test]
fn docs_view_handler_reads_fresh_docs_db_not_ctx_cozo() {
    with_temp_env_db("ARCHON_DOCS_DB_PATH", |path| {
        let db = test_docs_db_at(path);
        seed_doc(&db);
        drop(db);
        let stale_ctx_db = Arc::new(test_docs_db());
        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(stale_ctx_db).build();

        DocsViewHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let [TuiEvent::OpenViewRows { view_id, rows }] = events.as_slice() else {
            panic!("expected OpenViewRows, got {events:?}");
        };
        assert_eq!(*view_id, ViewId::Docs);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "doc-slash");
        assert!(rows[0].detail.contains("hash-slash"));
    });
}

#[test]
fn learning_view_handler_reads_configured_learning_db_not_ctx_cozo() {
    with_temp_env_db("ARCHON_LEARNING_DB_PATH", |path| {
        let db = test_learning_db_at(path);
        seed_learning_proposal(&db);
        drop(db);
        let stale_ctx_db = Arc::new(test_learning_db());
        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(stale_ctx_db).build();

        LearningViewHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let [TuiEvent::OpenViewRows { view_id, rows }] = events.as_slice() else {
            panic!("expected OpenViewRows, got {events:?}");
        };
        assert_eq!(*view_id, ViewId::Learning);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "proposal-slash");
        assert_eq!(rows[0].status, "Pending");
    });
}

#[test]
fn docs_status_reads_fresh_docs_db_not_ctx_cozo() {
    with_temp_env_db("ARCHON_DOCS_DB_PATH", |path| {
        let db = test_docs_db_at(path);
        seed_doc(&db);
        drop(db);
        let stale_ctx_db = Arc::new(test_docs_db());
        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(stale_ctx_db).build();

        DocsViewHandler
            .execute(&mut ctx, &[String::from("status")])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        let text = match &events[0] {
            TuiEvent::TextDelta(text) => text,
            other => panic!("expected TextDelta, got {other:?}"),
        };
        assert!(text.contains("Total sources: 1"));
        assert!(text.contains("Processed:     1"));
        assert!(text.contains("Total chunks:  1"));
    });
}

#[test]
fn docs_chunks_reads_fresh_docs_db_not_ctx_cozo() {
    with_temp_env_db("ARCHON_DOCS_DB_PATH", |path| {
        let db = test_docs_db_at(path);
        seed_doc(&db);
        drop(db);
        let stale_ctx_db = Arc::new(test_docs_db());
        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(stale_ctx_db).build();

        DocsViewHandler
            .execute(
                &mut ctx,
                &[String::from("chunks"), String::from("doc-slash")],
            )
            .unwrap();
        let events = drain_tui_events(&mut rx);
        let text = match &events[0] {
            TuiEvent::TextDelta(text) => text,
            other => panic!("expected TextDelta, got {other:?}"),
        };
        assert!(text.contains("chunk-slash"));
        assert!(text.contains("pages 1-1"));
    });
}

fn with_temp_env_db<F>(key: &'static str, f: F)
where
    F: FnOnce(&Path),
{
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var_os(key);
    let path = PathBuf::from(format!(
        "/tmp/evidence-view-{key}-{}.db",
        uuid::Uuid::new_v4()
    ));
    // SAFETY: ENV_LOCK serialises this module's environment mutation tests.
    unsafe {
        std::env::set_var(key, &path);
    }
    f(&path);
    // SAFETY: same lock-protected scope as above; restore original env.
    unsafe {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

fn test_docs_db() -> DbInstance {
    let path = format!("/tmp/test-docs-slash-{}.db", uuid::Uuid::new_v4());
    test_docs_db_at(Path::new(&path))
}

fn test_docs_db_at(path: &Path) -> DbInstance {
    let db = DbInstance::new("sqlite", path.to_str().unwrap_or(""), "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    db
}

fn test_learning_db() -> DbInstance {
    let path = format!("/tmp/test-learning-slash-{}.db", uuid::Uuid::new_v4());
    test_learning_db_at(Path::new(&path))
}

fn test_learning_db_at(path: &Path) -> DbInstance {
    let db = DbInstance::new("sqlite", path.to_str().unwrap_or(""), "").unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    db
}

fn seed_learning_proposal(db: &DbInstance) {
    archon_learning::store::insert_behaviour_proposal(
        db,
        &archon_learning::models::BehaviourProposal {
            proposal_id: "proposal-slash".into(),
            workspace_id: "workspace-slash".into(),
            manifest_kind: archon_learning::models::BehaviourManifestKind::RetrievalProfile,
            current_version: "v1".into(),
            proposed_version: "v2".into(),
            diff: "increase exact-search weight".into(),
            evidence_ids: vec!["le-1".into()],
            risk_level: archon_learning::models::RiskLevel::Low,
            policy_decision: archon_learning::models::PolicyDecision::PendingApproval,
            status: archon_learning::models::ProposalStatus::Pending,
            created_at: "2026-05-04T00:00:00Z".into(),
        },
    )
    .unwrap();
}

fn seed_doc(db: &DbInstance) {
    archon_docs::store::insert_doc_source(
        db,
        &SourceDocument {
            document_id: "doc-slash".into(),
            source_path: "/tmp/slash.md".into(),
            media_type: "text/markdown".into(),
            content_hash: "hash-slash".into(),
            discovered_at: "2026-05-04T00:00:00Z".into(),
            status: DocumentStatus::Processed,
        },
    )
    .unwrap();
    archon_docs::store::insert_chunk(
        db,
        &ChunkArtifact {
            chunk_id: "chunk-slash".into(),
            document_id: "doc-slash".into(),
            artifact_id: "artifact-slash".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "slash source of truth content".into(),
            content_hash: "chunk-hash".into(),
            embedding_status: "pending".into(),
        },
    )
    .unwrap();
}
