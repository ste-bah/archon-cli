//! A check's verdict at a commit and the failure signature that tells one
//! failure from another (Batch J, J2), split from `acceptance_regression`
//! for size.

use serde::{Deserialize, Serialize};

/// A check's verdict at a commit, as observed and cached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Verdict {
    /// `true`: it passed. `false`: it failed, how not recorded.
    Held(bool),
    /// It failed, with this [`failure_signature`] (Batch J2).
    Failed(String),
}

impl Verdict {
    pub fn passed(&self) -> bool {
        matches!(self, Self::Held(true))
    }

    /// Whether this is the failure the tip's `signature` names: the same
    /// signature, compared past what differs from run to run (Batch J2's
    /// rule: any other failure is GOOD for the search). Unknown (empty) on
    /// either side matches.
    pub fn fails_as(&self, signature: &str) -> bool {
        match self {
            Self::Held(passed) => !passed,
            Self::Failed(seen) => {
                seen.is_empty() || signature.is_empty() || comparable(seen) == comparable(signature)
            }
        }
    }
}

/// A signature as compared: its exit code, then its line with digit runs
/// masked (timings, counts) and `x=/path` words elided, cut to the first
/// 160 characters (the round's stored excerpt clips a long line past its
/// first 200 bytes).
fn comparable(signature: &str) -> String {
    let (exit, line) = signature.split_once(' ').unwrap_or((signature, ""));
    let mut masked = String::new();
    for word in line.split(' ') {
        if !masked.is_empty() {
            masked.push(' ');
        }
        if word.contains("=/") {
            masked.push_str("<path>");
            continue;
        }
        for c in word.chars() {
            if !c.is_ascii_digit() {
                masked.push(c);
            } else if !masked.ends_with('#') {
                masked.push('#');
            }
        }
    }
    format!("{exit} {}", masked.chars().take(160).collect::<String>())
}

/// What a failing check's output says, as the checks sharing a probe are
/// matched by: its exit code and the last line of its stderr (else its
/// stdout), with whitespace collapsed and absolute paths (a scratch's own
/// temporary directories) elided. Empty -- matching nothing -- when the
/// check printed nothing.
pub fn failure_signature(exit_code: Option<i32>, stderr: &str, stdout: &str) -> String {
    let last = |text: &str| {
        text.lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string)
    };
    let Some(line) = last(stderr).or_else(|| last(stdout)) else {
        return String::new();
    };
    let words: Vec<&str> = line
        .split_whitespace()
        .map(|word| {
            if word
                .trim_start_matches(['"', '\'', '`', '('])
                .starts_with('/')
            {
                "<path>"
            } else {
                word
            }
        })
        .collect();
    format!("{exit_code:?} {}", words.join(" "))
}
