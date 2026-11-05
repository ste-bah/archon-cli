//! Fatal expansion diagnostics are evidence even when an EXIT trap masks status.
use regex::Regex;
use std::sync::LazyLock;

pub(crate) fn runtime_unset(command: &str, _code: i32, diagnostic: &str, _bound: &[&str]) -> bool {
    static BASH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:/[^\s:]*/)?(?:sh|bash): line (\d+): ([A-Za-z_][A-Za-z0-9_]*): (.+)$")
            .unwrap()
    });
    static DASH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:/[^\s:]*/)?(?:sh|dash): (\d+): ([A-Za-z_][A-Za-z0-9_]*): (.+)$").unwrap()
    });
    let Some(c) = BASH
        .captures(diagnostic.trim_end())
        .or_else(|| DASH.captures(diagnostic.trim_end()))
    else {
        return false;
    };
    let Some(line) = c[1].parse::<usize>().ok().and_then(|n| n.checked_sub(1)) else {
        return false;
    };
    let Some(source) = command.lines().nth(line) else {
        return false;
    };
    let name = &c[2];
    let required =
        source.contains(&format!("${{{name}:?")) || source.contains(&format!("${{{name}?"));
    if &c[3] != "unbound variable" && &c[3] != "parameter not set" && !required {
        return false;
    }
    // No authoring exemptions: bindings, guards, traps and dynamic constructs
    // cannot overrule the executing shell's evidence. Only source attribution
    // matters. Scan just the named line, so unrelated opaque regions are harmless.
    reads_name(source, name)
}

// Attribution deliberately has no static-analysis failure path. An unrelated
// substitution or an error-message expansion must not hide a fatal read.
fn reads_name(source: &str, name: &str) -> bool {
    let bytes = source.as_bytes();
    let (mut i, mut single, mut double) = (0, false, false);
    while i < bytes.len() {
        let ch = bytes[i];
        if single {
            if ch == b'\'' {
                single = false;
            }
        } else if ch == b'\\' {
            i += 1;
        } else if ch == b'\'' && !double {
            single = true;
        } else if ch == b'"' {
            double = !double;
        } else if ch == b'#' && !double && (i == 0 || b" \t;|&()".contains(&bytes[i - 1])) {
            break;
        } else if ch == b'$' {
            let mut start = i + 1;
            if bytes.get(start) == Some(&b'{') {
                start += 1;
            }
            let mut end = start;
            while bytes
                .get(end)
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == b'_')
            {
                end += 1;
            }
            if &bytes[start..end] == name.as_bytes() {
                return true;
            }
        }
        i += 1;
    }
    false
}
