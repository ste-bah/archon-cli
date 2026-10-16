//! Shell fragments for moving inputs and detecting transient recreation.

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

pub(super) fn watch_prologue() -> &'static str {
    // mtime is user-settable, so ctime is required to catch recreation even
    // when a check restores the parent mtime. GNU stat's %z preserves its
    // highest-resolution ctime; BSD stat exposes seconds via %c. On Windows
    // Git Bash/MSYS, use stat's %z (Cygwin maps st_ctime to NTFS ChangeTime);
    // if it cannot be read, unavailable is treated as recreated.
    // macOS ctime has second precision, so wait for a timestamp tick.
    "__archon_dir_state() { stat -c '%Y:%i:%z' -- \"$1\" 2>/dev/null || stat -f '%m:%i:%c' \"$1\" 2>/dev/null || printf '%s' unavailable; }\n__archon_moved_any=0\n"
}

pub(super) fn move_inputs(inputs: &[String], tag: &str, moved_prefix: &str) -> String {
    let mut script = String::new();
    for input in inputs {
        let mut prefix = String::new();
        let mut plain = Vec::new();
        for part in input.split('/') {
            prefix = if prefix.is_empty() {
                part.to_string()
            } else {
                format!("{prefix}/{part}")
            };
            plain.push(format!("[ ! -L {} ]", quote(&prefix)));
        }
        let path = quote(input);
        script.push_str(&format!(
            "if {} && [ -e {path} ]; then mv -- {path} {path}.{tag} 2>/dev/null && {{ __archon_moved_any=1; printf '%s%s\\n' {} {path} >&2; }}; fi\n",
            plain.join(" && "),
            quote(moved_prefix),
        ));
    }
    script
}

pub(super) fn snapshot_parents(inputs: &[String], tag: &str) -> String {
    let mut script = String::new();
    for (index, input) in inputs.iter().enumerate() {
        let path = quote(input);
        script.push_str(&format!(
            "if [ -e {path}.{tag} ] || [ -L {path}.{tag} ]; then __archon_parent_{index}={}; __archon_state_{index}=$(__archon_dir_state \"$__archon_parent_{index}\"); fi\n",
            quote(&parent_path(input)),
        ));
    }
    script
}

pub(super) fn wait_for_timestamp_tick(inputs: &[String]) -> String {
    let unavailable = (0..inputs.len())
        .map(|index| format!("__archon_state_{index}=unavailable; "))
        .collect::<String>();
    format!("if [ \"$__archon_moved_any\" = 1 ]; then sleep 1 || {{ {unavailable}}}; fi\n")
}

pub(super) fn report_recreated(inputs: &[String], recreated_prefix: &str) -> String {
    let mut script = String::new();
    for (index, input) in inputs.iter().enumerate() {
        let path = quote(input);
        script.push_str(&format!(
            "if [ \"${{__archon_state_{index}+set}}\" = set ] && {{ [ \"$__archon_state_{index}\" = unavailable ] || [ \"$(__archon_dir_state \"$__archon_parent_{index}\")\" != \"$__archon_state_{index}\" ]; }}; then printf '%s%s\\n' {} {path} >&2; fi\n",
            quote(recreated_prefix),
        ));
    }
    script
}

fn parent_path(input: &str) -> String {
    match input.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((parent, _)) => parent.to_string(),
        None => ".".to_string(),
    }
}
