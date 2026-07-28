//! Query engine tests.

use super::*;
use crate::kb::schema::ensure_kb_schema;

fn test_db() -> cozo::DbInstance {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    ensure_kb_schema(&db).unwrap();
    db
}

fn insert_test_node(db: &cozo::DbInstance, id: &str, ntype: &str, title: &str, content: &str) {
    let mut params = BTreeMap::new();
    params.insert("nid".into(), DataValue::from(id));
    params.insert("ntype".into(), DataValue::from(ntype));
    params.insert("title".into(), DataValue::from(title));
    params.insert("content".into(), DataValue::from(content));
    params.insert("ts".into(), DataValue::from(1000.0));
    db.run_script(
        "?[node_id, node_type, source, domain_tag, title, content, \
         content_hash, chunk_index, created_at, updated_at] <- \
         [[$nid, $ntype, 'test', '', $title, $content, '', 0, $ts, $ts]] \
         :put kb_nodes { node_id => node_type, source, domain_tag, title, \
         content, content_hash, chunk_index, created_at, updated_at }",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();
}

fn insert_test_edge(db: &cozo::DbInstance, src: &str, tgt: &str, etype: &str) {
    let edge_id = format!("edge-{}", uuid::Uuid::new_v4());
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(edge_id.as_str()));
    params.insert("src".into(), DataValue::from(src));
    params.insert("tgt".into(), DataValue::from(tgt));
    params.insert("etype".into(), DataValue::from(etype));
    params.insert("ts".into(), DataValue::from(1000.0));
    db.run_script(
        "?[edge_id, source_node_id, target_node_id, edge_type, created_at] <- \
         [[$eid, $src, $tgt, $etype, $ts]] \
         :put kb_edges { edge_id => source_node_id, target_node_id, edge_type, \
         created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();
}

#[tokio::test]
async fn test_query_engine_empty_db() {
    let db = test_db();
    let engine = QueryEngine::new(db);
    let opts = QaQueryOptions::default();
    let result = engine.query("what is Rust?", &opts).await.unwrap();
    assert!(result.answer.contains("Insufficient context"));
    assert!(result.sources.is_empty());
    assert!(result.filed_node_id.is_none());
}

#[tokio::test]
async fn test_search_nodes_finds_matching() {
    let db = test_db();
    insert_test_node(
        &db,
        "n1",
        "raw",
        "Rust Programming",
        "Rust is a systems language.",
    );
    insert_test_node(&db, "n2", "raw", "Python Basics", "Python is interpreted.");

    let engine = QueryEngine::new(db);
    let results = engine.search_nodes("Rust", 10, None).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.node_id, "n1");
    assert!(results[0].score > 0.0);
}

#[tokio::test]
async fn search_nodes_matches_case_insensitively() {
    let db = test_db();
    insert_test_node(&db, "n1", "raw", "Rust Programming", "A systems language.");
    insert_test_node(
        &db,
        "n2",
        "raw",
        "Systems Programming",
        "The RuSt language.",
    );

    let engine = QueryEngine::new(db);
    let results = engine.search_nodes("rust", 10, None).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|result| result.node.node_id == "n1"));
    assert!(results.iter().any(|result| result.node.node_id == "n2"));
    assert!(results.iter().all(|result| result.score > 0.0));
}

#[tokio::test]
async fn test_search_nodes_answer_penalty() {
    let db = test_db();
    insert_test_node(
        &db,
        "raw1",
        "raw",
        "Rust guide",
        "Learn Rust programming today.",
    );
    insert_test_node(
        &db,
        "ans1",
        "answer",
        "Rust guide",
        "Learn Rust programming today.",
    );

    let engine = QueryEngine::new(db);
    let results = engine.search_nodes("Rust", 10, None).unwrap();

    assert_eq!(results.len(), 2);
    let raw_result = results
        .iter()
        .find(|r| r.node.node_type == KbNodeType::Raw)
        .unwrap();
    let ans_result = results
        .iter()
        .find(|r| r.node.node_type == KbNodeType::Answer)
        .unwrap();

    assert!(
        raw_result.score > ans_result.score,
        "Raw ({}) should score higher than Answer ({})",
        raw_result.score,
        ans_result.score
    );
    let expected_ans_score = raw_result.score * 0.9;
    assert!(
        (ans_result.score - expected_ans_score).abs() < 0.01,
        "Answer score {} should be ~0.9x of raw score {}",
        ans_result.score,
        raw_result.score
    );
}

#[test]
fn file_answer_truncates_multibyte_question_on_character_boundaries() {
    let db = test_db();
    let engine = QueryEngine::new(db.clone());
    let question: String = (0..101)
        .map(|offset| char::from_u32(0x4e00 + offset).unwrap())
        .collect();
    let synth_answer = SynthesizedAnswer {
        answer_text: "Unicode title answer.".to_string(),
        source_citations: vec![],
    };

    let filed_id = engine.file_answer(&question, &synth_answer, &[]).unwrap();

    let mut params = BTreeMap::new();
    params.insert("nid".into(), DataValue::from(filed_id.as_str()));
    let result = db
        .run_script(
            "?[title] := *kb_nodes{node_id, title}, node_id = $nid",
            params,
            ScriptMutability::Immutable,
        )
        .unwrap();
    let expected_title = format!("{}...", question.chars().take(97).collect::<String>());
    assert_eq!(result.rows[0][0].get_str(), Some(expected_title.as_str()));
}

#[tokio::test]
async fn test_file_answer_creates_node() {
    let db = test_db();
    insert_test_node(&db, "src1", "raw", "Source Doc", "Some source content.");
    insert_test_node(&db, "src2", "raw", "Another Doc", "More source content.");

    let engine = QueryEngine::new(db.clone());
    let synth_answer = SynthesizedAnswer {
        answer_text: "This is the synthesized answer.".to_string(),
        source_citations: vec![],
    };

    let filed_id = engine
        .file_answer(
            "What is the topic?",
            &synth_answer,
            &["src1".into(), "src2".into()],
        )
        .unwrap();

    assert!(filed_id.starts_with("answer-"));

    let mut params = BTreeMap::new();
    params.insert("nid".into(), DataValue::from(filed_id.as_str()));
    let result = db
        .run_script(
            "?[node_type, content] := *kb_nodes{node_id, node_type, content}, node_id = $nid",
            params,
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "answer");
    assert!(
        result.rows[0][1]
            .get_str()
            .unwrap()
            .contains("synthesized answer")
    );

    let mut edge_params = BTreeMap::new();
    edge_params.insert("nid".into(), DataValue::from(filed_id.as_str()));
    let edges = db
        .run_script(
            "?[target_node_id, edge_type] := *kb_edges{source_node_id, target_node_id, edge_type}, \
             source_node_id = $nid",
            edge_params,
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(edges.rows.len(), 2);
}

#[tokio::test]
async fn duplicate_answer_reuses_the_hash_owner() {
    let db = test_db();
    let engine = QueryEngine::new(db.clone());
    let answer = SynthesizedAnswer {
        answer_text: "Same answer text.".to_string(),
        source_citations: vec![],
    };

    let first = engine
        .file_answer("First question", &answer, &["source-one".into()])
        .unwrap();
    let second = engine
        .file_answer("Second question", &answer, &["source-two".into()])
        .unwrap();

    assert_eq!(second, first);
    let provenance = db
        .run_script(
            "?[target_node_id] := *kb_edges{source_node_id, target_node_id, edge_type}, \
             source_node_id = $nid, edge_type = 'DerivedFrom'",
            {
                let mut params = BTreeMap::new();
                params.insert("nid".into(), DataValue::from(first.as_str()));
                params
            },
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(provenance.rows.len(), 2);
    assert!(
        provenance
            .rows
            .iter()
            .any(|row| row[0].get_str() == Some("source-one"))
    );
    assert!(
        provenance
            .rows
            .iter()
            .any(|row| row[0].get_str() == Some("source-two"))
    );
    let answers = db
        .run_script(
            "?[node_id] := *kb_nodes{node_id, node_type}, node_type = 'answer'",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(answers.rows.len(), 1);
    let hashes = db
        .run_script(
            "?[node_id] := *kb_content_hashes{content_hash, node_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(hashes.rows.len(), 1);
    assert_eq!(hashes.rows[0][0].get_str(), Some(first.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_answers_with_same_hash_keep_one_owner() {
    let db = test_db();
    let first = QueryEngine::new(db.clone());
    let second = QueryEngine::new(db.clone());
    let answer = SynthesizedAnswer {
        answer_text: "Concurrent answer text.".to_string(),
        source_citations: vec![],
    };
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let runtime = tokio::runtime::Handle::current();
    let first_barrier = barrier.clone();
    let second_barrier = barrier.clone();
    let first_answer = answer.clone();
    let second_answer = answer.clone();
    let first_runtime = runtime.clone();
    let second_runtime = runtime.clone();

    let first_task = tokio::task::spawn_blocking(move || {
        first_barrier.wait();
        first_runtime.block_on(async { first.file_answer("First", &first_answer, &[]) })
    });
    let second_task = tokio::task::spawn_blocking(move || {
        second_barrier.wait();
        second_runtime.block_on(async { second.file_answer("Second", &second_answer, &[]) })
    });
    let (first, second) = tokio::join!(first_task, second_task);
    let first = first.unwrap().unwrap();
    let second = second.unwrap().unwrap();

    assert_eq!(first, second);
    let answers = db
        .run_script(
            "?[node_id] := *kb_nodes{node_id, node_type}, node_type = 'answer'",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(answers.rows.len(), 1);
    let mappings = db
        .run_script(
            "?[node_id] := *kb_content_hashes{content_hash, node_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(mappings.rows.len(), 1);
    assert_eq!(mappings.rows[0][0].get_str(), Some(first.as_str()));
}

#[tokio::test]
async fn answer_hash_blocks_later_duplicate_raw_ingest() {
    let db = test_db();
    let engine = QueryEngine::new(db.clone());
    let answer = SynthesizedAnswer {
        answer_text: "Answer that must reserve the hash.".to_string(),
        source_citations: vec![],
    };
    engine.file_answer("Question", &answer, &[]).unwrap();
    let ingester = crate::kb::ingest::Ingester::new(db.clone()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("duplicate.md");
    std::fs::write(&path, &answer.answer_text).unwrap();

    let result = ingester.ingest_text(&path, "test").await.unwrap();

    assert_eq!(result.nodes_created, 0);
}

#[tokio::test]
async fn test_gather_graph_context_follows_edges() {
    let db = test_db();
    insert_test_node(&db, "n1", "raw", "Main Doc", "Main content about Rust.");
    insert_test_node(&db, "c1", "concept", "Ownership", "Rust ownership model.");
    insert_test_node(&db, "b1", "raw", "Backlink Source", "References main doc.");

    insert_test_edge(&db, "n1", "c1", "ConceptOf");
    insert_test_edge(&db, "b1", "n1", "Backlink");

    let engine = QueryEngine::new(db);
    let scored = vec![ScoredKbNode {
        node: KbNode {
            node_id: "n1".into(),
            node_type: KbNodeType::Raw,
            source: "test".into(),
            domain_tag: String::new(),
            title: "Main Doc".into(),
            content: "Main content about Rust.".into(),
            content_hash: String::new(),
            chunk_index: 0,
            created_at: 1000.0,
            updated_at: 1000.0,
        },
        score: 0.8,
    }];

    let ctx = engine.gather_graph_context(&scored).unwrap();
    assert_eq!(ctx.primary_nodes.len(), 1);
    assert_eq!(ctx.related_concepts.len(), 1);
    assert_eq!(ctx.related_concepts[0].node_id, "c1");
    assert_eq!(ctx.backlinks.len(), 1);
    assert_eq!(ctx.backlinks[0].node_id, "b1");
}

#[tokio::test]
async fn test_synthesize_answer_without_llm() {
    let db = test_db();
    let engine = QueryEngine::new(db);

    let context = GraphContext {
        primary_nodes: vec![ScoredKbNode {
            node: KbNode {
                node_id: "n1".into(),
                node_type: KbNodeType::Raw,
                source: "test".into(),
                domain_tag: String::new(),
                title: "Test Doc".into(),
                content: "Test content here.".into(),
                content_hash: String::new(),
                chunk_index: 0,
                created_at: 1000.0,
                updated_at: 1000.0,
            },
            score: 0.9,
        }],
        ..Default::default()
    };

    let result = engine
        .synthesize_answer("What is this?", &context)
        .await
        .unwrap();
    assert!(result.answer_text.contains("1 knowledge base sources"));
    assert!(result.answer_text.contains("Test Doc"));
    assert_eq!(result.source_citations.len(), 1);
    assert_eq!(result.source_citations[0].node_id, "n1");
}

#[tokio::test]
async fn test_query_full_flow() {
    let db = test_db();
    insert_test_node(
        &db,
        "doc1",
        "raw",
        "Ownership in Rust",
        "Rust uses ownership to manage memory safely without garbage collection.",
    );
    insert_test_node(
        &db,
        "doc2",
        "raw",
        "Rust Borrowing",
        "Borrowing in Rust allows references without taking ownership.",
    );

    let engine = QueryEngine::new(db);
    let opts = QaQueryOptions {
        top_k: 5,
        file_answer: false,
        include_graph_context: true,
        node_type_filter: None,
    };

    let result = engine.query("Rust", &opts).await.unwrap();
    assert!(!result.answer.is_empty());
    assert!(!result.answer.contains("Insufficient context"));
    assert!(!result.sources.is_empty());
    assert_eq!(result.sources.len(), 2);
    assert!(result.search_duration_ms < 500);
}

#[tokio::test]
async fn test_filed_answer_ranked_below_original() {
    let db = test_db();
    insert_test_node(
        &db,
        "original",
        "raw",
        "Rust Safety",
        "Rust ensures memory safety through its type system.",
    );
    insert_test_node(
        &db,
        "filed-ans",
        "answer",
        "Rust Safety",
        "Rust ensures memory safety through its type system.",
    );

    let engine = QueryEngine::new(db);
    let results = engine.search_nodes("Rust", 10, None).unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].node.node_type,
        KbNodeType::Raw,
        "Raw node should rank above answer node"
    );
    assert_eq!(results[1].node.node_type, KbNodeType::Answer);
    assert!(results[0].score > results[1].score);
}
