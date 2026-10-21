//! Shell wrapper for running a mutation check and reporting its exit status.

use super::Markers;

#[cfg(test)]
pub fn wrap(
    original: &str,
    inputs: &[String],
    live_roots: &[&std::path::Path],
    markers: &Markers,
) -> String {
    let roots = vec![None; inputs.len()];
    wrap_with_paths(original, inputs, inputs, &roots, live_roots, markers)
}

pub fn wrap_with_paths(
    original: &str,
    _inputs: &[String],
    _move_paths: &[String],
    _move_roots: &[Option<String>],
    _live_roots: &[&std::path::Path],
    markers: &Markers,
) -> String {
    let status_name = format!("archon-mutation-status-{}:", markers.nonce());
    let script = format!(
        "__archon_observed_signal=none\ntrap '__archon_observed_signal=HUP' HUP\ntrap '__archon_observed_signal=INT' INT\ntrap '__archon_observed_signal=QUIT' QUIT\ntrap '__archon_observed_signal=TERM' TERM\n(\n{original}\n)\n__archon_status=$?\nif [ \"$__archon_status\" -gt 128 ] 2>/dev/null; then __archon_signal=$(($__archon_status - 128)); else __archon_signal=none; fi\nprintf '%s %s:%s:%s:%s\\n' '{}' '{}' \"$__archon_status\" \"$__archon_signal\" \"$__archon_observed_signal\"\nexit $__archon_status\n",
        quote(&status_name),
        quote(&markers.tag()),
    );
    #[cfg(test)]
    {
        let paths = _inputs
            .iter()
            .enumerate()
            .map(|(index, named)| {
                let path = _move_paths.get(index).unwrap_or(named);
                let root = _move_roots
                    .get(index)
                    .and_then(Option::as_deref)
                    .unwrap_or("");
                format!(
                    "{}:{}",
                    test_hex(path.as_bytes()),
                    test_hex(root.as_bytes())
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let live = _live_roots
            .iter()
            .map(|root| test_hex(root.to_string_lossy().as_bytes()))
            .collect::<Vec<_>>()
            .join(",");
        let mut script = script;
        script.insert_str(
            0,
            &format!(
                "# archon-test-mutation-v1:{}:{}:{}\n",
                markers.nonce(),
                paths,
                live
            ),
        );
        script
    }
    #[cfg(not(test))]
    script
}

#[cfg(test)]
fn test_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}
