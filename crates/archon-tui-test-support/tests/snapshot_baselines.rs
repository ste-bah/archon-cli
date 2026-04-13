use archon_tui_test_support::fixtures::{
    error_toast_buffer, idle_prompt_buffer, inflight_agent_buffer, modal_overlay_buffer,
    splash_screen_buffer,
};
use archon_tui_test_support::insta_wrapper::assert_buffer_snapshot;

#[test]
fn splash_screen() {
    let buf = splash_screen_buffer();
    assert_buffer_snapshot("splash_screen", &buf);
}

#[test]
fn idle_prompt() {
    let buf = idle_prompt_buffer("hello world");
    assert_buffer_snapshot("idle_prompt", &buf);
}

#[test]
fn inflight_agent() {
    let buf = inflight_agent_buffer("general-purpose", 1234);
    assert_buffer_snapshot("inflight_agent", &buf);
}

#[test]
fn error_toast() {
    let buf = error_toast_buffer("connection refused");
    assert_buffer_snapshot("error_toast", &buf);
}

#[test]
fn modal_overlay() {
    let buf = modal_overlay_buffer("Confirm", "are you sure?");
    assert_buffer_snapshot("modal_overlay", &buf);
}
