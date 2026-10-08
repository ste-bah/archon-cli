//! A real slash handler, index search, baseline and mutant verifier; no model download.
use super::*;
struct ConstantEmbedder;
impl archon_memory::embedding::EmbeddingProvider for ConstantEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, archon_memory::types::MemoryError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0, 0.0]).collect())
    }
    fn dimensions(&self) -> usize {
        4
    }
}

pub(super) fn execute(root: &Path, user: &Path, resolved: ArchonConfig, expected: bool) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    let original = "fn fixture() {\n    println!(\"fixture\");\n}\n";
    std::fs::write(root.join("src/fixture.rs"), original).unwrap();
    std::fs::write(
        root.join("PRD.md"),
        "- REQ-AB-001: fixture must fail closed.\n",
    )
    .unwrap();
    std::fs::write(root.join("tasks/TASK-AB-001.md"), "```yaml\ntask_id: TASK-AB-001\nimplements: [REQ-AB-001]\nrequired_tools: []\n```\n\n## Files Expected to Change\n\n- `src/fixture.rs`\n\n## Focused Tests\n\n- `sh verify-fixture.sh`\n").unwrap();
    std::fs::write(root.join("verify-fixture.sh"), "test -n \"${FIXTURE_API_KEY-}\" || { echo 'FIXTURE_API_KEY environment variable is not set'; exit 1; }\ntest -z \"${OPENAI_API_KEY-}\" || exit 2\ntouch forwarded-marker\ngrep -q 'archon falsification probe' src/fixture.rs && exit 3\nexit 0\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "src/fixture.rs"],
        vec![
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=fixture",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        assert!(
            archon_shell::spawn::command("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    std::fs::write(root.join("evidence.json"), r#"{"commands_run":[{"kind":"test","command":"sh verify-fixture.sh","status":"succeeded","exit_code":0,"output_summary":"fixture"}]}"#).unwrap();
    let paths = archon_topology::trace::TopologyPaths::for_project(root);
    let trace_path = paths.trace_jsonl("fixture");
    std::fs::create_dir_all(trace_path.parent().unwrap()).unwrap();
    let record = archon_topology::trace::TraceRecord::new(
        "2026-10-06T00:00:00Z",
        "fixture",
        archon_topology::trace::TraceKind::FileRead,
    )
    .with_node("TASK-AB-001")
    .with_reads(vec![archon_topology::ir::WriteTarget::Path(
        "src/fixture.rs".into(),
    )]);
    std::fs::write(
        trace_path,
        format!("{}\n", serde_json::to_string(&record).unwrap()),
    )
    .unwrap();
    let db_path = root.join("index.db");
    {
        let db = cozo::DbInstance::new("sqlite", db_path.to_str().unwrap(), "").unwrap();
        let indexer = archon_leann::indexer::Indexer::new(
            db.clone(),
            archon_leann::indexer::EmbeddingConfig {
                provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
                dimension: 4,
            },
            None,
        )
        .unwrap();
        indexer.ensure_schema().unwrap();
        db.run_script(r#"?[chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding] <- [["fixture", "src/fixture.rs", "rust", 2, 2, "fixture fail closed", "fixture", 0.0, [1.0, 0.0, 0.0, 0.0]]]
:put code_chunks {chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding}"#, Default::default(), cozo::ScriptMutability::Mutable).unwrap();
    }
    let (mut ctx, _rx) = crate::command::test_support::CtxBuilder::new().build();
    ctx.working_dir = Some(root.into());
    ctx.config_path = Some(user.into());
    ctx.workflow_config = Some(resolved);
    let args: Vec<String> = [
        "trace",
        "--prd",
        "PRD.md",
        "--tasks",
        "tasks",
        "--evidence",
        "evidence.json",
        "--graph",
        "fixture",
        "--leann-db",
        "index.db",
        "--falsify",
        "--json",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let result = crate::command::requirement_trace::with_test_embedder(
        &db_path,
        Arc::new(ConstantEmbedder),
        || crate::command::requirement_trace::RequirementsHandler.execute(&mut ctx, &args),
    );
    result.unwrap();
    assert_eq!(
        root.join("forwarded-marker").exists(),
        expected,
        "TUI falsify ignored the resolved allowlist"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("src/fixture.rs")).unwrap(),
        original
    );
}
