//! Fatal expansion evidence and conservative whole-script defect attribution.
use regex::Regex;
use std::sync::LazyLock;

static BASH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:/[^\s:]*/)?(?:sh|bash): (?:line \d+: )?([A-Za-z_][A-Za-z0-9_]*): (.*)$")
        .unwrap()
});
static DASH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:/[^\s:]*/)?(?:sh|dash): \d+: (?:eval: )?([A-Za-z_][A-Za-z0-9_]*): (.*)$")
        .unwrap()
});

fn diagnostic(line: &str) -> Option<regex::Captures<'_>> {
    BASH.captures(line).or_else(|| DASH.captures(line))
}
fn required(command: &str, name: &str) -> bool {
    command.contains(&format!("${{{name}:?")) || command.contains(&format!("${{{name}?"))
}
fn fatal(message: &str) -> bool {
    matches!(
        message,
        "unbound variable"
            | "parameter null or not set"
            | "parameter not set"
            | "parameter not set or null"
    )
}

/// No source attribution is needed to reject a potentially masked abort.
pub(crate) fn never_pass(command: &str, code: i32, stderr: &str) -> bool {
    code == 0
        && stderr.lines().any(|line| {
            diagnostic(line)
                .is_some_and(|c| fatal(&c[2]) || required(command, &c[1]) || command.is_empty())
        })
}

/// A required-value abort is a product verdict. Only a never-bound ordinary
/// read, found by the same conservative authoring rule, is a script defect.
pub(crate) fn runtime_unset(command: &str, _code: i32, line: &str, bound: &[&str]) -> bool {
    let Some(c) = diagnostic(line) else {
        return false;
    };
    let name = &c[1];
    matches!(&c[2], "unbound variable" | "parameter not set")
        && !required(command, name)
        && !bound.contains(&name)
        && super::unset_reads_with_bound(command, bound)
            .iter()
            .any(|read| read.name == name)
}
