use super::*;
use crate::command::registry::CommandHandler;
use crate::command::test_support::*;
use archon_tui::app::TuiEvent;

fn render_args(args: &[&str]) -> String {
    let handler = ProvidersHandler;
    let (mut ctx, mut rx) = make_bug_ctx();
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    handler.execute(&mut ctx, &args).unwrap();
    let events = drain_tui_events(&mut rx);
    assert_eq!(events.len(), 1);
    match events.into_iter().next().unwrap() {
        TuiEvent::TextDelta(text) => text,
        other => panic!("expected TextDelta, got {other:?}"),
    }
}

#[test]
fn execute_capabilities_lists_codex_agentic_surface_support() {
    let body = render_args(&["capabilities"]);
    assert!(body.contains("Archon provider capability matrix"));
    assert!(body.contains("| `openai-codex` |"));
    assert!(body.contains("provider-neutral pipelines"));
    assert!(body.contains("subagents, /btw"));
}

#[test]
fn cli_handle_capabilities_renders_without_error() {
    handle_providers(
        Some(ProvidersAction::Capabilities),
        &archon_core::config::ArchonConfig::default(),
    )
    .expect("capabilities output");
}

#[test]
fn description_and_aliases() {
    let h = ProvidersHandler;
    assert!(!h.description().is_empty());
    assert_eq!(h.aliases(), &[] as &[&'static str]);
}

#[test]
#[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
fn providers_dispatches_via_registry() {
    // Gate 5 smoke: Registry::get("providers") must return Some,
    // and execute must emit a single TextDelta with both section
    // headers + the 40-total marker.
    use crate::command::registry::default_registry;

    let registry = default_registry();
    let handler = registry
        .get("providers")
        .expect("providers must be registered in default_registry()");

    let (mut ctx, mut rx) = make_bug_ctx();
    handler.execute(&mut ctx, &[]).unwrap();
    let events = drain_tui_events(&mut rx);
    assert_eq!(events.len(), 1);
    let body = match &events[0] {
        TuiEvent::TextDelta(s) => s.clone(),
        other => panic!("expected TextDelta, got {:?}", other),
    };
    assert!(body.contains("37 total"));
    assert!(body.contains("NATIVE (6)"));
    assert!(body.contains("OPENAI-COMPAT (31)"));
}
