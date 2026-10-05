//! The programs a check's shell text runs (Issue 328 round 2): its simple
//! commands, each reduced to the program it starts and that program's
//! arguments, read the way a POSIX shell reads them -- quotes, comments,
//! heredoc bodies, redirections, variable assignments, `$(...)` and
//! pipelines -- without running anything. A word the shell would expand
//! (`$`, a glob, `~`) is kept, marked unresolvable.

use std::sync::LazyLock;

use regex::Regex;

/// Shell builtins and keywords: never looked up on the search path.
const BUILTINS: &[&str] = &[
    ":", ".", "[", "[[", "]]", "alias", "bg", "break", "builtin", "cd", "command", "continue",
    "declare", "dirs", "echo", "eval", "exec", "exit", "export", "false", "fg", "getopts", "hash",
    "jobs", "kill", "let", "local", "popd", "printf", "pushd", "pwd", "read", "readonly", "return",
    "set", "shift", "shopt", "source", "test", "times", "trap", "true", "type", "typeset",
    "ulimit", "umask", "unalias", "unset", "wait", "if", "then", "else", "elif", "fi", "do",
    "done", "while", "until", "for", "in", "case", "esac", "function", "select", "{", "}", "!",
    "time",
];

/// Keywords that open or join a command without starting a program.
const PREFIXES: &[&str] = &[
    "{", "}", "!", "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "esac",
];

/// Keywords whose command starts no program of its own.
const NO_PROGRAM: &[&str] = &["for", "case", "select", "function", "in"];

/// Words that start another program given as their arguments.
const WRAPPERS: &[&str] = &["exec", "command", "nohup", "env", "time", "nice", "timeout"];

/// A shell function the text defines (`name() {`, `function name`).
static FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(?:^|[\s;{(&|])(?:function\s+([A-Za-z_][\w.-]*)|([A-Za-z_][\w.-]*)\s*\(\s*\))")
        .expect("static pattern")
});

/// One simple command: the program it starts (`None` when it starts none,
/// or the program is a builtin, a function or an expansion) and its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Simple {
    pub(super) program: Option<String>,
    pub(super) args: Vec<String>,
}

/// Every simple command of `text`, in order (see the module docs).
pub(super) fn simple_commands(text: &str) -> Vec<Simple> {
    let functions: Vec<String> = FUNCTION
        .captures_iter(text)
        .filter_map(|c| c.get(1).or_else(|| c.get(2)))
        .map(|m| m.as_str().to_string())
        .collect();
    let mut out = Vec::new();
    for words in lex(text) {
        let mut words = words.into_iter().peekable();
        let mut program = None;
        while let Some(word) = words.next() {
            if is_assignment(&word) || PREFIXES.contains(&word.as_str()) {
                continue;
            }
            if NO_PROGRAM.contains(&word.as_str()) {
                break;
            }
            if WRAPPERS.contains(&word.as_str()) {
                // Their options, and `timeout`'s duration, are no program.
                while words
                    .peek()
                    .is_some_and(|w| w.starts_with('-') || is_assignment(w) || is_duration(w))
                {
                    words.next();
                }
                continue;
            }
            program = Some(word);
            break;
        }
        let Some(word) = program else { continue };
        let starts_none =
            BUILTINS.contains(&word.as_str()) || functions.contains(&word) || expands(&word);
        out.push(Simple {
            program: (!starts_none).then_some(word),
            args: words.collect(),
        });
    }
    out
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn is_duration(word: &str) -> bool {
    word.trim_end_matches(['s', 'm', 'h', 'd'])
        .parse::<f64>()
        .is_ok()
}

/// Whether the shell would expand `word` before running it.
pub(super) fn expands(word: &str) -> bool {
    word.contains(['$', '*', '?', '~', '`'])
}

/// The words of each simple command of `text`, quotes removed.
fn lex(text: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let (mut commands, mut words, mut word) = (Vec::new(), Vec::new(), String::new());
    let mut heredocs: Vec<(String, bool)> = Vec::new();
    let mut at = 0;
    let end_word = |word: &mut String, words: &mut Vec<String>| {
        if !word.is_empty() {
            words.push(std::mem::take(word));
        }
    };
    while at < chars.len() {
        let c = chars[at];
        match c {
            '\\' if at + 1 < chars.len() => {
                if chars[at + 1] != '\n' {
                    word.push(chars[at + 1]);
                }
                at += 2;
                continue;
            }
            '\'' => {
                let close = (at + 1..chars.len()).find(|&i| chars[i] == '\'');
                let stop = close.unwrap_or(chars.len());
                word.extend(&chars[at + 1..stop]);
                at = stop + 1;
                continue;
            }
            '"' => {
                at += 1;
                while at < chars.len() && chars[at] != '"' {
                    if chars[at] == '\\' && at + 1 < chars.len() {
                        at += 1;
                    }
                    word.push(chars[at]);
                    at += 1;
                }
                at += 1;
                continue;
            }
            '#' if word.is_empty() => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
                continue;
            }
            '<' if chars.get(at + 1) == Some(&'<') && chars.get(at + 2) != Some(&'<') => {
                at += 2;
                let dash = chars.get(at) == Some(&'-');
                at += usize::from(dash);
                while chars.get(at).is_some_and(|c| *c == ' ' || *c == '\t') {
                    at += 1;
                }
                let mut tag = String::new();
                while let Some(&c) = chars.get(at) {
                    if c.is_whitespace() || ";&|<>()".contains(c) {
                        break;
                    }
                    if c != '\'' && c != '"' {
                        tag.push(c);
                    }
                    at += 1;
                }
                heredocs.push((tag, dash));
                continue;
            }
            '>' | '<' => {
                // A redirection: its fd, operator and target start nothing.
                if word.chars().all(|c| c.is_ascii_digit()) {
                    word.clear();
                } else {
                    end_word(&mut word, &mut words);
                }
                while chars
                    .get(at)
                    .is_some_and(|c| matches!(c, '>' | '<' | '&' | '|'))
                {
                    at += 1;
                }
                while chars.get(at).is_some_and(|c| *c == ' ' || *c == '\t') {
                    at += 1;
                }
                while let Some(&c) = chars.get(at) {
                    if c.is_whitespace() || ";&|<>()".contains(c) {
                        break;
                    }
                    at += 1;
                }
                continue;
            }
            '\n' | ';' | '&' | '|' | '(' | ')' | '`' => {
                end_word(&mut word, &mut words);
                if !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
                at += 1;
                if c == '\n' {
                    at = skip_heredocs(&chars, at, &mut heredocs);
                }
                continue;
            }
            '$' if chars.get(at + 1) == Some(&'(') => {
                end_word(&mut word, &mut words);
                if !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
                at += 2;
                continue;
            }
            c if c.is_whitespace() => end_word(&mut word, &mut words),
            c => word.push(c),
        }
        at += 1;
    }
    end_word(&mut word, &mut words);
    if !words.is_empty() {
        commands.push(words);
    }
    commands
}

/// Past every pending heredoc body starting at `at`.
fn skip_heredocs(chars: &[char], mut at: usize, pending: &mut Vec<(String, bool)>) -> usize {
    for (tag, dash) in std::mem::take(pending) {
        loop {
            let start = at;
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
            let line: String = chars[start..at].iter().collect();
            at = (at + 1).min(chars.len());
            let line = if dash {
                line.trim_start_matches('\t')
            } else {
                &line
            };
            if line == tag || at >= chars.len() {
                break;
            }
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn programs(text: &str) -> Vec<Option<String>> {
        simple_commands(text)
            .into_iter()
            .map(|c| c.program)
            .collect()
    }

    #[test]
    fn programs_are_read_as_a_shell_reads_them() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            programs("cd sub && FOO=1 cargo test -q 2>/dev/null | tee out.txt"),
            vec![None, some("cargo"), some("tee")]
        );
        assert_eq!(
            programs("bash scripts/new.sh 2>/dev/null"),
            vec![some("bash")]
        );
        assert_eq!(
            programs("python3 - <<'PY'\nimport os\nmissing_tool x\nPY\njq . a.json"),
            vec![some("python3"), some("jq")]
        );
        assert_eq!(
            programs("helper() { grep -q x a; }\nhelper && ! grep -rn 'error!' src # jq"),
            vec![None, some("grep"), None, some("grep")]
        );
        assert_eq!(
            programs("env -i PATH=/x timeout 30 ./bin/tool $(git rev-parse HEAD)"),
            vec![some("./bin/tool"), some("git")]
        );
        assert_eq!(programs("$RUNNER check"), vec![None]);
    }
}
