use super::*;

/// List branch, non-empty graph: emits the byte-identical
/// formatted-list TextDelta with the `.1`-precision score format
/// per the shipped slash.rs:603-611 format loop.
#[test]
fn execute_list_non_empty_emits_formatted_textdelta() {
    let graph = make_graph();
    // Seed two rules with known text and scores. RulesEngine
    // sorts by score descending so rule_a (score 80.0) must come
    // before rule_b (score 30.0) in the output.
    {
        let engine = RulesEngine::new(graph.as_ref());
        let r_a = engine
            .add_rule("rule alpha", RuleSource::UserDefined)
            .expect("seed rule a");
        let r_b = engine
            .add_rule("rule beta", RuleSource::SystemDefault)
            .expect("seed rule b");
        graph
            .apply_importance_delta(&r_a.id, 30.0, "fixture:rules-list:alpha")
            .expect("set score a");
        graph
            .apply_importance_delta(&r_b.id, -20.0, "fixture:rules-list:beta")
            .expect("set score b");
    }
    // Snapshot the sorted rules for byte-exact assertion.
    let sorted = RulesEngine::new(graph.as_ref())
        .get_rules_sorted()
        .expect("sorted list");
    assert_eq!(sorted.len(), 2, "seeded 2 rules");
    let expected = {
        let mut out = format!("\n{} behavioral rules:\n\n", sorted.len());
        for r in &sorted {
            let id_short = &r.id[..8.min(r.id.len())];
            out.push_str(&format!(
                "  [{id_short}] (score: {:.1}) {}\n",
                r.score, r.text
            ));
        }
        out
    };

    let memory: Arc<dyn MemoryTrait> = graph.clone();
    let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
    let h = RulesHandler::new();
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_ok(),
        "list non-empty must return Ok(()), got: {res:?}"
    );

    let ev = rx.try_recv().expect("formatted TextDelta must be emitted");
    match ev {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, expected,
                "list non-empty TextDelta must be byte-identical \
                 to the shipped slash.rs:603-611 format (count + \
                 word 'behavioral rules' + colon + blank line + \
                 two-space bracket + one-space '(score: {{:.1}})' + \
                 one-space text + single newline per rule)"
            );
            // Defence-in-depth: verify `.1` precision is honoured.
            assert!(
                text.contains("(score: 80.0)"),
                "expected '.1' precision '80.0' in output, got: {text}"
            );
            assert!(
                text.contains("(score: 30.0)"),
                "expected '.1' precision '30.0' in output, got: {text}"
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta with formatted list, \
             got: {other:?}"
        ),
    }
}

/// Edit branch, success: seed a rule, resolve it by ID prefix,
/// update its text, and verify the byte-identical
/// `"\nRule updated: {new_text}\n"` TextDelta.
#[test]
fn execute_edit_success_emits_rule_updated() {
    let graph = make_graph();
    let rule_id: String = {
        let engine = RulesEngine::new(graph.as_ref());
        engine
            .add_rule("old text", RuleSource::UserDefined)
            .expect("seed rule")
            .id
    };
    let id_prefix: String = rule_id[..8.min(rule_id.len())].to_string();

    let memory: Arc<dyn MemoryTrait> = graph.clone();
    let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
    let h = RulesHandler::new();
    let args = vec![
        "edit".to_string(),
        id_prefix,
        "new".to_string(),
        "text".to_string(),
    ];
    let res = h.execute(&mut ctx, &args);
    assert!(res.is_ok(), "edit success must return Ok(()), got: {res:?}");

    let ev = rx.try_recv().expect("TextDelta must be emitted");
    match ev {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, "\nRule updated: new text\n",
                "edit success TextDelta must be byte-identical to \
                 the shipped slash.rs:635-639 literal (note: the \
                 new_text here is 'new text' — two tokens joined \
                 via rest.splitn(2, ' ') which preserves the \
                 single whitespace)"
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta with rule-updated \
             literal, got: {other:?}"
        ),
    }

    // Verify the rule text was actually updated in the graph.
    let rules = RulesEngine::new(graph.as_ref())
        .get_rules_sorted()
        .expect("resorted list");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].text, "new text",
        "edit branch must persist the new text via \
         RulesEngine::update_rule"
    );
}

/// Remove branch, success: seed a rule, resolve it by ID prefix,
/// remove it, and verify the byte-identical
/// `"\nRule removed: {rule.text}\n"` TextDelta.
#[test]
fn execute_remove_success_emits_rule_removed() {
    let graph = make_graph();
    let rule_id: String = {
        let engine = RulesEngine::new(graph.as_ref());
        engine
            .add_rule("doomed rule", RuleSource::UserDefined)
            .expect("seed rule")
            .id
    };
    let id_prefix: String = rule_id[..8.min(rule_id.len())].to_string();

    let memory: Arc<dyn MemoryTrait> = graph.clone();
    let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
    let h = RulesHandler::new();
    let args = vec!["remove".to_string(), id_prefix];
    let res = h.execute(&mut ctx, &args);
    assert!(
        res.is_ok(),
        "remove success must return Ok(()), got: {res:?}"
    );

    let ev = rx.try_recv().expect("TextDelta must be emitted");
    match ev {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, "\nRule removed: doomed rule\n",
                "remove success TextDelta must be byte-identical \
                 to the shipped slash.rs:671-676 literal (uses \
                 positional arg for rule.text)"
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta with rule-removed \
             literal, got: {other:?}"
        ),
    }

    // Verify the rule was actually removed from the graph.
    let rules = RulesEngine::new(graph.as_ref())
        .get_rules_sorted()
        .expect("resorted list");
    assert!(
        rules.is_empty(),
        "remove branch must delete the rule via \
         RulesEngine::remove_rule"
    );
}

/// Dispatcher-integration (happy-path list-empty). Narrow
/// `RegistryBuilder::new()` wires ONLY `/rules` with
/// `RulesHandler::new()`, then
/// `Dispatcher::dispatch(&mut ctx, "/rules")` routes through the
/// real alias+primary pipeline with a real in-memory
/// MemoryGraph. Asserts the dispatcher's end-to-end wiring
/// (parser → registry → handler.execute) delivers the byte-
/// identical no-rules TextDelta.
#[test]
fn dispatcher_routes_slash_rules_with_memory_emits_textdelta() {
    let mut builder = RegistryBuilder::new();
    builder.insert_primary("rules", Arc::new(RulesHandler::new()));
    let registry = Arc::new(builder.build());
    let dispatcher = Dispatcher::new(registry);

    let graph = make_graph();
    let memory: Arc<dyn MemoryTrait> = graph.clone();
    let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
    let res = dispatcher.dispatch(&mut ctx, "/rules");
    assert!(
        res.is_ok(),
        "dispatcher.dispatch must return Ok(()) for the list-empty \
         path, got: {res:?}"
    );

    let ev = rx.try_recv().expect("TextDelta must be emitted");
    match ev {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, "\nNo behavioral rules.\n",
                "dispatcher must deliver the byte-identical \
                 no-rules TextDelta through the full parser → \
                 registry → handler pipeline"
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta with no-rules literal, \
             got: {other:?}"
        ),
    }
}

/// Dispatcher-integration (error-surfacing path). Narrow
/// `RegistryBuilder::new()` wires ONLY `/rules`, dispatches
/// `"/rules"` with `memory: None`, and asserts that
/// `Dispatcher::dispatch` surfaces the handler's Err. Mirrors
/// the B18 /recall `dispatcher_routes_slash_recall_without_memory
/// _returns_err` precedent.
#[test]
fn dispatcher_routes_slash_rules_without_memory_returns_err() {
    let mut builder = RegistryBuilder::new();
    builder.insert_primary("rules", Arc::new(RulesHandler::new()));
    let registry = Arc::new(builder.build());
    let dispatcher = Dispatcher::new(registry);

    let (mut ctx, _rx) = make_rules_ctx(None);
    let res = dispatcher.dispatch(&mut ctx, "/rules");
    assert!(
        res.is_err(),
        "dispatcher.dispatch must surface handler Err when \
         memory is None (dispatcher forwards the Err verbatim), \
         got: {res:?}"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.contains("memory") && msg.contains("build_command_context"),
        "Err message must mention both 'memory' and \
         'build_command_context', got: {msg}"
    );
}
