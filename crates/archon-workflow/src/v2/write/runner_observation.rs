//! Diagnostic-only options for plain runner invocations. Shell compositions
//! retain their authored output contract; unknown runners stay verbatim.
use std::borrow::Cow;

pub(super) fn command(command: &str) -> Cow<'_, str> {
    let words: Vec<_> = command.split_whitespace().collect();
    if !words.iter().all(|word| {
        word.chars()
            .all(|ch| ch.is_alphanumeric() || "_./:=,-".contains(ch))
    }) {
        return Cow::Borrowed(command);
    }
    let Some(program) = words.first() else {
        return Cow::Borrowed(command);
    };
    let name = program.rsplit('/').next().unwrap_or(program);
    let (prefix, flag): (usize, &str) = match (name, words.get(1).copied(), words.get(2).copied()) {
        ("go", Some("test"), _) if !words.iter().any(|word| word.starts_with("-json")) => {
            (2, "-json")
        }
        ("pytest" | "py.test", _, _) if !words.contains(&"-v") && !words.contains(&"--verbose") => {
            (1, "-v")
        }
        ("python" | "python3", Some("-m"), Some("pytest" | "unittest"))
            if !words.contains(&"-v") && !words.contains(&"--verbose") =>
        {
            (3, "-v")
        }
        _ => return Cow::Borrowed(command),
    };
    let mut observed = words[..prefix].to_vec();
    observed.push(flag);
    observed.extend(&words[prefix..]);
    Cow::Owned(observed.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_preserve_every_selector_and_leave_shell_contracts_alone() {
        for (authored, observed) in [
            ("go test ./...", "go test -json ./..."),
            ("pytest a.py b.py", "pytest -v a.py b.py"),
            (
                "python3 -m unittest test_a test_b",
                "python3 -m unittest -v test_a test_b",
            ),
        ] {
            assert_eq!(command(authored), observed);
        }
        for authored in [
            "go test -json ./pkg",
            "go test ./pkg | grep ok",
            "custom-runner a b",
            "pytest -v a.py",
            "printf 'go test ./pkg'",
        ] {
            assert_eq!(command(authored), authored);
        }
    }
}
