//! Conservative nounset authoring gate. Runtime evidence is classified separately.
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;
#[path = "acceptance_check_shell_unset_runtime.rs"]
mod runtime;
#[path = "acceptance_check_shell_unset_scan.rs"]
mod scan;
pub(crate) use runtime::runtime_unset;

#[derive(Debug, PartialEq, Eq)]
pub struct UnsetRead {
    pub name: String,
    pub line: usize,
    pub lines: Vec<usize>,
}
impl UnsetRead {
    pub fn finding(&self) -> String {
        format!(
            "line {} expands shell variable '{}' under set -u, but the check never sets it or uses a default form; set {} or remove that line",
            self.lines
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            self.name,
            self.name
        )
    }
}
fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static shell pattern")
}
static OPAQUE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?:^|[;\n|&()]|then |do )\s*(?:eval|source|\.|unset|alias|readarray|mapfile|getopts|let)\s|`|\bprintf\s+-v\b|\b(?:declare|typeset)\s+-[^;\n]*n",
    )
});
static DISABLE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:^|[;\n])\s*set\s+\+[^\n;]*u|\bset\s+\+o\s+nounset"));
static ENABLE: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?:^|[;\n])\s*set\s+(?:-[A-Za-z]+\s+)*(?:-[a-zA-Z]*u[a-zA-Z]*\b|-o\s+nounset\b)")
});
static CONDITIONAL: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:if|then|for|while|until|case|function)\b"));
static ASSIGN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:^|[^A-Za-z0-9_])([A-Za-z_][A-Za-z0-9_]*)\s*\+?=[^=]"));
static DECLARE: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:export|local|readonly|declare|typeset|read)\b([^;\n]*)"));
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[A-Za-z_][A-Za-z0-9_]*"));
static FOR: LazyLock<Regex> = LazyLock::new(|| re(r"\bfor\s+([A-Za-z_][A-Za-z0-9_]*)\b"));
static GUARD: LazyLock<Regex> =
    LazyLock::new(|| re(r"\[\[\s+-v\s+([A-Za-z_][A-Za-z0-9_]*)\s*\]\]\s*&&"));

pub fn unset_reads(command: &str) -> Vec<UnsetRead> {
    unset_reads_with_bound(command, &[])
}
pub fn unset_reads_with_bound(command: &str, bound: &[&str]) -> Vec<UnsetRead> {
    let Some((code, reads, defaults)) = scan::scan(command) else {
        return Vec::new();
    };
    if OPAQUE.is_match(&code) || DISABLE.is_match(&code) {
        return Vec::new();
    }
    let Some(enabled) = ENABLE.find(&code) else {
        return Vec::new();
    };
    let before = &code[..enabled.start()];
    if before.contains(['(', ')', '{', '}'])
        || before.contains("&&")
        || before.contains("||")
        || CONDITIONAL.is_match(before)
    {
        return Vec::new();
    }
    let mut bindings: BTreeSet<String> = defaults;
    for c in ASSIGN.captures_iter(&code) {
        bindings.insert(c[1].into());
    }
    for c in FOR.captures_iter(&code) {
        bindings.insert(c[1].into());
    }
    for c in DECLARE.captures_iter(&code) {
        for name in WORD.find_iter(&c[1]) {
            bindings.insert(name.as_str().into());
        }
        if c[0].starts_with("read") {
            bindings.insert("REPLY".into());
        }
    }
    let mut grouped: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (name, offset) in reads {
        let line = command[..offset].bytes().filter(|&ch| ch == b'\n').count() + 1;
        let guarded = GUARD.captures_iter(&code).any(|c| {
            c[1] == name
                && c.get(0).unwrap().end() <= offset
                && !code[c.get(0).unwrap().end()..offset].contains(['\n', ';'])
        });
        if offset < enabled.end()
            || bindings.contains(&name)
            || guarded
            || crate::acceptance_check_environment::shell_bound_name(&name, bound)
        {
            continue;
        }
        let lines = grouped.entry(name).or_default();
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    grouped
        .into_iter()
        .map(|(name, lines)| UnsetRead {
            name,
            line: lines[0],
            lines,
        })
        .collect()
}
#[cfg(test)]
#[path = "acceptance_check_shell_unset_tests.rs"]
mod tests;
