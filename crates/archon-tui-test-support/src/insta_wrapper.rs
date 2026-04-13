//! Thin wrapper over `insta` for consistent snapshot naming.

use ratatui::buffer::Buffer;

/// Render a ratatui Buffer to a deterministic text grid (one String per row,
/// joined with '\n'), then assert against a named insta snapshot.
pub fn assert_buffer_snapshot(name: &str, buffer: &Buffer) {
    let area = buffer.area();
    let mut rows: Vec<String> = Vec::with_capacity(area.height as usize);
    for y in 0..area.height {
        let mut line = String::with_capacity(area.width as usize);
        for x in 0..area.width {
            let cell = &buffer[(x, y)];
            line.push_str(cell.symbol());
        }
        // Trim trailing whitespace so snapshots stay deterministic regardless
        // of terminal padding. Internal spaces are preserved.
        while line.ends_with(' ') {
            line.pop();
        }
        rows.push(line);
    }
    let rendered = rows.join("\n");
    insta::assert_snapshot!(name, rendered);
}

/// Replace non-deterministic substrings (RFC3339 timestamps, "<N>ms" durations,
/// UUIDs) with stable placeholders so snapshots remain reproducible.
pub fn redact_dynamic(text: &str) -> String {
    // RFC3339 timestamps: 2026-04-11T12:00:00Z (with optional fractional + offset)
    let ts_re = regex_lite::Regex::new(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})",
    )
    .unwrap();
    let after_ts = ts_re.replace_all(text, "<TS>");

    let dur_re = regex_lite::Regex::new(r"\d+\s*ms").unwrap();
    let after_dur = dur_re.replace_all(&after_ts, "<DUR>");

    let uuid_re = regex_lite::Regex::new(
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
    )
    .unwrap();
    let after_uuid = uuid_re.replace_all(&after_dur, "<UUID>");

    after_uuid.into_owned()
}

#[cfg(test)]
mod tests {
    use super::redact_dynamic;

    #[test]
    fn redact_dynamic_masks_timestamps_durations_uuids() {
        let input =
            "elapsed 1234ms at 2026-04-11T12:00:00Z uuid=01234567-89ab-cdef-0123-456789abcdef";
        let out = redact_dynamic(input);
        assert!(out.contains("<TS>"), "timestamp not redacted: {out}");
        assert!(out.contains("<DUR>"), "duration not redacted: {out}");
        assert!(out.contains("<UUID>"), "uuid not redacted: {out}");
        assert!(!out.contains("1234ms"));
        assert!(!out.contains("2026-04-11T12:00:00Z"));
    }
}
