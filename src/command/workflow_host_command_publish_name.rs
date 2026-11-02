//! Stable component of a run-owned publication path.
pub(super) fn sanitize_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "call".to_string()
    } else {
        value
    }
}
