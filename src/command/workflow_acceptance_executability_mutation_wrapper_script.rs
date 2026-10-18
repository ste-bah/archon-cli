//! Shell fragments for moving inputs and detecting transient recreation.

use std::path::Path;

use super::Markers;

pub fn wrap(original: &str, inputs: &[String], live_roots: &[&Path], markers: &Markers) -> String {
    let quote = |text: &str| format!("'{}'", text.replace('\'', r"'\''"));
    let tag = markers.tag();
    let mut script = String::new();
    script.push_str(watch_prologue());
    for root in live_roots {
        let root = quote(&root.to_string_lossy());
        script.push_str(&format!(
            "case \"$(pwd -P)/\" in {root}/*) printf '%s\\n' {} >&2; exit {};; esac\n",
            quote(&markers.guard()),
            super::GUARD_STATUS
        ));
    }
    let sibling_prunes = archon_workflow::acceptance_sibling_paths::scan(original)
        .into_iter()
        .filter_map(|sibling| sibling.path.parent().map(Path::to_path_buf))
        .filter(|parent| {
            parent
                .components()
                .any(|part| part == std::path::Component::ParentDir)
        })
        .map(|parent| {
            format!(
                "-path {} -prune -o ",
                quote(&format!("./{}", parent.display()))
            )
        })
        .collect::<String>();
    // The command runs from its copied project/repository root. Prune any
    // sibling roots it names before searching that root for stale artifacts.
    let sweep = format!(
        "find . \\( -name .git -o -name target \\) -prune -o {sibling_prunes}-name '*.archon-mutated-*' -print"
    );
    script.push_str(&format!(
        "{sweep} | while IFS= read -r __archon_left; do __archon_base=\"${{__archon_left%.archon-mutated-*}}\"; if [ -e \"$__archon_base\" ] || [ -L \"$__archon_base\" ]; then rm -rf -- \"$__archon_left\"; else mv -- \"$__archon_left\" \"$__archon_base\"; fi; done\n"
    ));
    // The wrapper hides the original command from the site copy builder.
    for sibling in archon_workflow::acceptance_sibling_paths::scan(original) {
        let path = quote(&sibling.path.to_string_lossy());
        script.push_str(&format!("test -e {path} || :\n"));
    }
    script.push_str(&move_inputs(inputs, &tag, &markers.moved_prefix()));
    script.push_str(&snapshot_parents(inputs, &tag));
    script.push_str(&wait_for_timestamp_tick(inputs));
    script.push_str(&format!("(\n{original}\n)\n__archon_status=$?\n"));
    script.push_str(&report_recreated(inputs, &markers.recreated_prefix()));
    for input in inputs {
        let path = quote(input);
        script.push_str(&format!(
            "if [ -e {path} ] || [ -L {path} ]; then printf '%s%s\\n' {} {path} >&2; fi\n",
            quote(&markers.generated_prefix()),
        ));
    }
    for input in inputs.iter().rev() {
        let path = quote(input);
        script.push_str(&format!(
            "if [ -e {path}.{tag} ] || [ -L {path}.{tag} ]; then rm -rf -- {path}; mv -- {path}.{tag} {path}; fi\n"
        ));
    }
    script.push_str(&format!(
        "if [ -z \"$({sweep} | grep -F -- {} | head -n 1)\" ]; then printf '%s\\n' {}; fi\nexit $__archon_status\n",
        quote(&tag),
        quote(&markers.restored()),
    ));
    script
}

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
    if inputs.is_empty() {
        return String::new();
    }
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
