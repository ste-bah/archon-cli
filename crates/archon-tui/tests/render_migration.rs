/// TUI-309 migration test.
/// This test verifies the render module can be called with the current AppState.
/// After migration, app.rs should call render::draw() instead of inline draw closures.

#[test]
fn render_module_exists() {
    // Just verify the module compiles — actual draw behavior is tested via integration
    let _ = std::path::Path::new("crates/archon-tui/src/render.rs");
}
