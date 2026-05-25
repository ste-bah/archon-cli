use archon_cognitive::{
    ClassifyInput, CognitiveDecision, CognitiveStore, CognitiveSurface, SituationClassifier,
    ToolVerdict,
};
use cozo::{DbInstance, ScriptMutability};

#[test]
fn store_writes_compact_situation_and_decision_rows() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    let store = CognitiveStore::new(&db).expect("store");
    let situation = SituationClassifier.classify(ClassifyInput {
        user_text: "hello",
        session_id: "test-session",
        turn_number: 7,
        surface: CognitiveSurface::Tui,
    });
    store.put_situation(&situation).expect("put situation");
    let decision = CognitiveDecision::for_tool(
        &situation,
        "Bash",
        ToolVerdict::Suppress {
            reason: "trivial turn".to_owned(),
        },
    );
    store.put_decision(&decision).expect("put decision");

    let rows = db
        .run_script(
            "?[kind, hash] := *cognitive_situations{kind, user_text_hash: hash}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("query situations");
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0][0].get_str(), Some("greeting"));
    assert_ne!(rows.rows[0][1].get_str(), Some("hello"));

    let rows = db
        .run_script(
            "?[tool_name] := *cognitive_tool_decisions{tool_name}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("query decisions");
    assert_eq!(rows.rows[0][0].get_str(), Some("Bash"));
}
