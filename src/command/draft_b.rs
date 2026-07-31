#[cfg(test)]
mod draft_handler_tests {
    use super::*;
    use crate::command::model::ModelSnapshot;
    use crate::command::test_support::CtxBuilder;

    fn ctx_with(
        model: Option<&str>,
        wd: Option<&str>,
    ) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
        let mut b = CtxBuilder::new();
        if let Some(m) = model {
            b = b.with_model_snapshot(ModelSnapshot {
                current_model: m.to_string(),
            });
        }
        b = b.with_working_dir_opt(wd.map(PathBuf::from));
        b.build()
    }

    #[test]
    fn stashes_effect_with_session_model() {
        let (mut ctx, _rx) = ctx_with(Some("claude-opus-4-8"), Some("/proj"));
        DraftHandler
            .execute(&mut ctx, &["pack.json".to_string(), "out".to_string()])
            .unwrap();
        match ctx.pending_effect {
            Some(CommandEffect::RunDraft {
                ref pack,
                ref workdir,
                ref model,
                ref gate_config,
                ref cwd,
            }) => {
                assert_eq!(pack, &PathBuf::from("pack.json"));
                assert_eq!(workdir, &PathBuf::from("out"));
                assert_eq!(model, "claude-opus-4-8");
                assert!(gate_config.is_none());
                assert_eq!(cwd, &PathBuf::from("/proj"));
            }
            ref other => panic!("expected RunDraft, got {other:?}"),
        }
    }

    #[test]
    fn model_flag_overrides_session_model_and_takes_gate_config() {
        let (mut ctx, _rx) = ctx_with(Some("claude-opus-4-8"), Some("/proj"));
        DraftHandler
            .execute(
                &mut ctx,
                &[
                    "p.json".to_string(),
                    "out".to_string(),
                    "--model".to_string(),
                    "claude-fable-5".to_string(),
                    "--gate-config".to_string(),
                    "g.json".to_string(),
                ],
            )
            .unwrap();
        match ctx.pending_effect {
            Some(CommandEffect::RunDraft {
                ref model,
                ref gate_config,
                ..
            }) => {
                assert_eq!(model, "claude-fable-5");
                assert_eq!(gate_config.as_deref(), Some(Path::new("g.json")));
            }
            ref other => panic!("expected RunDraft, got {other:?}"),
        }
    }

    #[test]
    fn usage_emitted_and_no_effect_when_missing_args() {
        let (mut ctx, mut rx) = ctx_with(Some("claude-opus-4-8"), Some("/proj"));
        DraftHandler
            .execute(&mut ctx, &["only-pack".to_string()])
            .unwrap();
        assert!(ctx.pending_effect.is_none());
        match rx.try_recv().expect("usage event") {
            TuiEvent::TextDelta(m) => assert!(m.contains("Usage: /draft")),
            other => panic!("expected usage TextDelta, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_import_roundtrips_through_store() {
        let dir = std::env::temp_dir().join(format!("archon-draft-prov-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chain = dir.join("provenance.jsonl");
        let _ = std::fs::remove_file(&chain);

        // Build a small FCDP chain via archon-draft's own recorder (distinct content →
        // distinct content hashes → distinct record ids).
        let art = dir.join("a.md");
        std::fs::write(&art, "movement plan body").unwrap();
        archon_draft::provenance::record(
            &chain,
            &art,
            "d1-plan",
            &serde_json::json!({"gates_run": []}),
        )
        .unwrap();
        std::fs::write(&art, "assembled draft body").unwrap();
        archon_draft::provenance::record(
            &chain,
            &art,
            "d2-assembled",
            &serde_json::json!({"words": 3}),
        )
        .unwrap();
        std::fs::write(&art, "revised draft body").unwrap();
        archon_draft::provenance::record(
            &chain,
            &art,
            "revision",
            &serde_json::json!({"cycle": 1}),
        )
        .unwrap();

        let db_file = dir.join("prov.db");
        // SAFETY: single-threaded test; no other thread reads the env concurrently.
        unsafe {
            std::env::set_var("ARCHON_PROV_DB_PATH", &db_file);
        }

        // No quotes → no corpus linkage attempted (hermetic: this db has no ingested corpus).
        let summary = import_provenance_to_store(&chain, "test-section", "claude-fable-5", &[], "")
            .unwrap()
            .expect("non-empty chain");
        assert_eq!(summary.records, 3);
        assert_eq!(summary.final_artifact, "test-section#02-revision");
        assert!(summary.cited.is_empty());
        let final_artifact = summary.final_artifact;

        let db = crate::command::store_paths::open_sqlite_db(&db_file, "provenance").unwrap();

        // trace the final artifact → export as W3C PROV JSON-LD → must reach the root
        let trace = archon_provenance::traverse::trace_artifact(&db, &final_artifact).unwrap();
        let jsonld = archon_provenance::export_w3c::export_trace_jsonld(&trace).to_string();
        assert!(
            jsonld.contains("test-section#00-d1-plan"),
            "trace/export must reach the root artifact: {jsonld}"
        );

        // verify runs end-to-end against the imported chain
        archon_provenance::verify::verify_artifact(&db, &final_artifact).unwrap();

        unsafe {
            std::env::remove_var("ARCHON_PROV_DB_PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_quote_strips_markup_and_surrounding_quotation() {
        // LaTeX quote markup (``…'') + \textit{…} → verbatim inner text that matches the corpus.
        assert_eq!(
            clean_quote("``the absorption of a virtual environment''"),
            "the absorption of a virtual environment"
        );
        assert_eq!(
            clean_quote("\\textit{always already} predetermined"),
            "always already predetermined"
        );
    }
}
