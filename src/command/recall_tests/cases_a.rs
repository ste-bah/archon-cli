use super::*;

#[test]
fn recall_handler_description_byte_identical_to_shipped() {
    assert_eq!(
        RecallHandler::new().description(),
        "Recall memories matching a query"
    );
}

/// R5: zero aliases. Shipped stub used the 2-arg
/// `declare_handler!` form (no aliases slice) and the Steven
/// directive at registry.rs:1302-1304 explicitly forbids adding
/// `recall` as an alias on any other handler.
#[test]
fn recall_handler_aliases_are_empty() {
    assert_eq!(RecallHandler::new().aliases(), &[] as &[&str]);
}

/// Empty args: emit the usage-error TuiEvent with the EXACT
/// byte-identity em-dash literal (U+2014, NOT a hyphen) and
/// return Ok(()). No memory lookup is performed (the empty-args
/// branch short-circuits BEFORE the memory check, matching the
/// shipped control flow at slash.rs:572).
#[test]
fn execute_with_empty_args_emits_usage_error_with_em_dash() {
    // memory: None is fine here — the empty-args branch
    // short-circuits before touching ctx.memory, matching shipped
    // control flow.
    let (mut ctx, mut rx) = make_recall_ctx(None);
    let h = RecallHandler::new();
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_ok(),
        "empty-args branch must return Ok(()) (event emission is \
         best-effort via try_send), got: {res:?}"
    );
    let ev = rx.try_recv().expect("usage error must be emitted");
    match ev {
        TuiEvent::Error(msg) => {
            assert_eq!(
                msg, "Usage: /recall <query> — search memories by keyword",
                "Usage error must be byte-identical to the shipped \
                 slash.rs:574-576 literal, INCLUDING the em-dash \
                 (U+2014) between '<query>' and 'search'. A \
                 hyphen-minus here is a byte-identity violation."
            );
            // Defence-in-depth: verify the em-dash byte-exactly.
            // U+2014 is 3 bytes in UTF-8: E2 80 94.
            assert!(
                msg.contains('\u{2014}'),
                "usage error must contain U+2014 EM DASH, got: {msg}"
            );
        }
        other => panic!(
            "expected TuiEvent::Error with em-dash usage literal, \
             got: {other:?}"
        ),
    }
}
