pub(super) fn now_iso8601() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // Simple ISO 8601-ish timestamp
    format!("{}Z", dur.as_secs())
}
