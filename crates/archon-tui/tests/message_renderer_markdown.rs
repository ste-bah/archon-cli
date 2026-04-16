use archon_tui::message_renderer::render_message;
use archon_tui::theme::intj_theme;

#[test]
fn basic_text() {
    let theme = intj_theme();
    let lines = render_message("hello", "user", &theme);
    assert!(!lines.is_empty());
}

#[test]
fn role_header_present() {
    let theme = intj_theme();
    let lines = render_message("test", "assistant", &theme);
    let first = &lines[0];
    let text = format!("{:?}", first);
    assert!(text.contains("Assistant"));
}