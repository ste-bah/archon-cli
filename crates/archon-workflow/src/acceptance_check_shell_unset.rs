//! Conservative nounset analysis shared by authoring and runtime classification.
//!
//! This is deliberately not a shell parser. Dynamic bindings, heredocs and
//! nested programs are left to execution. Policy bindings are not available at
//! every validation/classification call site: ambient names are exempt as a
//! conservative fallback (including host names a policy may forward). Explicit
//! policy-only bindings absent from the host remain a limitation of this gate.
use regex::Regex;
use std::collections::BTreeSet;

#[derive(Debug, PartialEq, Eq)]
pub struct UnsetRead {
    pub name: String,
    pub line: usize,
}

impl UnsetRead {
    pub fn finding(&self) -> String {
        format!(
            "line {} expands shell variable '{}' under set -u, but the check never sets it or uses a default form; set {} or remove that line",
            self.line, self.name, self.name
        )
    }
}

fn matches(pattern: &str, text: &str) -> bool {
    Regex::new(pattern)
        .expect("static shell pattern")
        .is_match(text)
}

fn environment_name(name: &str) -> bool {
    use crate::acceptance_check_environment::{DEFAULT_BOUND, PROXY_VARIABLES, SYSTEM_VARIABLES};
    [
        "HOME",
        "PATH",
        "PWD",
        "OLDPWD",
        "TMPDIR",
        "TMP",
        "TEMP",
        "USER",
        "LOGNAME",
        "SHELL",
        "SHLVL",
        "IFS",
        "OPTIND",
        "OPTARG",
        "PPID",
        "PID",
        "UID",
        "EUID",
        "BASH",
        "HOSTNAME",
        "HOSTTYPE",
        "MACHTYPE",
        "OSTYPE",
        "PS1",
        "PS2",
        "PS4",
        "ZSH_VERSION",
        "BASH_VERSION",
        "BASH_VERSINFO",
        "BASH_SOURCE",
        "LINENO",
        "RANDOM",
        "SECONDS",
        "_",
    ]
    .contains(&name)
        || DEFAULT_BOUND.contains(&name)
        || PROXY_VARIABLES.contains(&name)
        || SYSTEM_VARIABLES.contains(&name)
        || std::env::var_os(name).is_some()
}

/// Reads definitely outside single quotes/comments, in a check that explicitly
/// enables nounset. A binding anywhere exempts a name: branch-dependent reads
/// belong to execution, not to this rule. Positional/special parameters are not
/// names. Command substitutions and arithmetic are opaque to this analysis.
pub fn unset_reads(command: &str) -> Vec<UnsetRead> {
    let Some((code, reads, defaults)) = scan(command) else {
        return Vec::new();
    };
    if matches(
        r"(?:^|[\s;|&])(?:eval|source|\.|unset|alias|trap|readarray|mapfile|getopts|let)\s|<<|`",
        &code,
    ) || matches(r"\bprintf\s+-v\b|\b(?:declare|typeset)\s+-[^;\n]*n", &code)
        || matches(
            r"(?:^|[;\n])\s*set\s+\+[^\n;]*u|\bset\s+\+o\s+nounset",
            &code,
        )
    {
        return Vec::new();
    }
    let enable = Regex::new(
        r"(?:^|[;\n])\s*set\s+(?:-[A-Za-z]+\s+)*(?:-[a-zA-Z]*u[a-zA-Z]*\b|-o\s+nounset\b)",
    )
    .expect("static nounset pattern");
    let Some(enabled) = enable.find(&code) else {
        return Vec::new();
    };
    // Do not infer that an option command in a function, conditional or
    // subshell runs. The common unconditional preamble is provable here.
    let before = &code[..enabled.start()];
    if before.contains(['(', ')', '{', '}'])
        || before.contains("&&")
        || before.contains("||")
        || matches(r"\b(?:if|then|for|while|until|case|function)\b", before)
    {
        return Vec::new();
    }
    reads
        .into_iter()
        .filter_map(|(name, offset)| {
            if environment_name(&name) || defaults.contains(&name) || offset < enabled.end() {
                return None;
            }
            let n = regex::escape(&name);
            let bound = matches(&format!(r"(?:^|[^A-Za-z0-9_]){n}="), &code)
                || matches(
                    &format!(r"\b(?:export|local|readonly|declare|typeset)\b[^;\n]*\b{n}\b"),
                    &code,
                )
                || matches(&format!(r"\bfor\s+{n}\b"), &code)
                || matches(&format!(r"\bread\b[^;\n]*\b{n}\b"), &code);
            (!bound).then(|| UnsetRead {
                name,
                line: command[..offset].bytes().filter(|&b| b == b'\n').count() + 1,
            })
        })
        .collect()
}

type Scan = (String, Vec<(String, usize)>, BTreeSet<String>);

// Keep byte offsets and newlines while masking quoted literal text. Only the
// expansion collector sees double quotes, so quoted text cannot invent a set
// command or a binding. Unrecognized parameter syntax fails closed.
fn scan(command: &str) -> Option<Scan> {
    let b = command.as_bytes();
    let mut code = b.to_vec();
    let mut reads = Vec::new();
    let mut defaults = BTreeSet::new();
    let (mut i, mut single, mut double) = (0, false, false);
    while i < b.len() {
        let ch = b[i];
        if ch == b'\n' {
            i += 1;
            continue;
        }
        if single {
            code[i] = b' ';
            if ch == b'\'' {
                single = false;
            }
            i += 1;
            continue;
        }
        if ch == b'\\' {
            code[i] = b' ';
            i += 1;
            if i < b.len() {
                code[i] = b' ';
                i += 1;
            }
            continue;
        }
        if ch == b'\'' && !double {
            single = true;
            code[i] = b' ';
            i += 1;
            continue;
        }
        if ch == b'"' {
            double = !double;
            code[i] = b' ';
            i += 1;
            continue;
        }
        if ch == b'#' && !double && (i == 0 || b" \t\n;|&()".contains(&b[i - 1])) {
            while i < b.len() && b[i] != b'\n' {
                code[i] = b' ';
                i += 1;
            }
            continue;
        }
        if ch == b'`' {
            return None;
        }
        if ch == b'$' {
            let start = i;
            i += 1;
            if b.get(i) == Some(&b'(') {
                // Nested shell grammars/quotes are not inferred. Skip a simple
                // balanced substitution; complicated ones disable the rule.
                let arithmetic = b.get(i + 1) == Some(&b'(');
                let body = i;
                let mut depth = 0;
                while i < b.len() {
                    match b[i] {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        b'\'' | b'"' | b'`' | b'\\' => return None,
                        _ => {}
                    }
                    i += 1;
                }
                if depth != 0 {
                    return None;
                }
                if arithmetic
                    && (command[body..i].contains('=')
                        || command[body..i].contains("++")
                        || command[body..i].contains("--"))
                {
                    return None;
                }
            } else {
                let braced = b.get(i) == Some(&b'{');
                if braced {
                    i += 1;
                }
                let begin = i;
                if b.get(i)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
                {
                    i += 1;
                    while b
                        .get(i)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                    {
                        i += 1;
                    }
                    let name = command[begin..i].to_string();
                    if braced && b.get(i) != Some(&b'}') {
                        if b.get(i) == Some(&b':') {
                            i += 1;
                        }
                        if !b.get(i).is_some_and(|c| b"-=+".contains(c)) {
                            return None;
                        }
                        defaults.insert(name);
                        while i < b.len() && b[i] != b'}' {
                            if b"${\n".contains(&b[i]) {
                                return None;
                            }
                            i += 1;
                        }
                        if b.get(i) != Some(&b'}') {
                            return None;
                        }
                    } else {
                        reads.push((name, start));
                    }
                    if braced {
                        i += 1;
                    }
                } else if braced {
                    return None;
                }
            }
            for c in &mut code[start..i] {
                if *c != b'\n' {
                    *c = b' ';
                }
            }
            continue;
        }
        if double {
            code[i] = b' ';
        }
        i += 1;
    }
    if single || double {
        return None;
    }
    Some((String::from_utf8(code).ok()?, reads, defaults))
}

/// Only stdin diagnostics from shells the harness uses, with the observed
/// exit status. Dash uses status 2 and a different diagnostic from bash/sh.
pub(crate) fn runtime_unset(command: &str, code: i32, last: &str, bound: &[&str]) -> bool {
    let patterns = [
        (
            1,
            r"^(?:/[^\s:]*/)?(?:sh|bash): line (\d+): ([A-Za-z_][A-Za-z0-9_]*): unbound variable$",
        ),
        (
            2,
            r"^(?:/[^\s:]*/)?(?:sh|dash): (\d+): ([A-Za-z_][A-Za-z0-9_]*): parameter not set$",
        ),
    ];
    patterns.iter().any(|(status, pattern)| {
        if code != *status {
            return false;
        }
        let re = Regex::new(pattern).expect("static diagnostic pattern");
        let Some(c) = re.captures(last.trim_end()) else {
            return false;
        };
        let line = c[1].parse::<usize>().ok();
        unset_reads(command).iter().any(|read| {
            Some(read.line) == line && read.name == c[2] && !bound.contains(&read.name.as_str())
        })
    })
}

#[cfg(test)]
#[path = "acceptance_check_shell_unset_tests.rs"]
mod tests;
