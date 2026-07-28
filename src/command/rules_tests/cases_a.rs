use super::*;

#[test]
fn rules_handler_description_byte_identical_to_shipped() {
    assert_eq!(
        RulesHandler::new().description(),
        "List, edit, or remove behavioral rules"
    );
}

/// R5: zero aliases. Shipped stub used the 2-arg
/// `declare_handler!` form (no aliases slice) and AGS-817
/// shipped-wins preserves zero aliases.
#[test]
fn rules_handler_aliases_are_empty() {
    assert_eq!(RulesHandler::new().aliases(), &[] as &[&str]);
}

/// R6: when `memory` is None, execute returns Err whose message
/// mentions both `memory` and `build_command_context` so the
/// operator can trace the wiring bug. Mirrors the AGS-817 /memory
/// and B18 /recall precedent.
#[test]
fn execute_without_memory_returns_err() {
    let (mut ctx, _rx) = make_rules_ctx(None);
    let h = RulesHandler::new();
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_err(),
        "RulesHandler::execute with None memory must return Err \
         (builder contract violation), got: {res:?}"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.contains("memory"),
        "Err message must mention 'memory' so the operator can \
         trace the wiring bug, got: {msg}"
    );
    assert!(
        msg.contains("build_command_context"),
        "Err message must mention 'build_command_context' to pin \
         the owning builder, got: {msg}"
    );
}

/// List branch, empty graph: emits the byte-identical
/// `"\nNo behavioral rules.\n"` TextDelta.
#[test]
fn execute_list_empty_emits_no_rules_textdelta() {
    let graph = make_graph();
    let memory: Arc<dyn MemoryTrait> = graph.clone();
    let (mut ctx, mut rx) = make_rules_ctx(Some(memory));
    let h = RulesHandler::new();
    let res = h.execute(&mut ctx, &[]);
    assert!(res.is_ok(), "list-empty must return Ok(()), got: {res:?}");

    let ev = rx.try_recv().expect("TextDelta must be emitted");
    match ev {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, "\nNo behavioral rules.\n",
                "list-empty TextDelta must be byte-identical to \
                 the shipped slash.rs:598-600 literal"
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta with no-rules literal, \
             got: {other:?}"
        ),
    }
}
