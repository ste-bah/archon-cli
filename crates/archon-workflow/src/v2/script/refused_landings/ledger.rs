//! The run's append-only record of every refused landing the host took back
//! out (`write-coordination/refused-landings.jsonl`), and the finding a
//! later attempt at the same tasks is given from it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Most refusals one preamble quotes.
const PREAMBLE_REFUSALS: usize = 6;

/// One decision about one refused landing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedLandingRevert {
    /// When it was decided, in nanoseconds since the epoch.
    pub at: i64,
    /// `commit`, `project_input`, `copy` or `serial`.
    pub kind: String,
    /// The landing: a commit sha, `<stage>/<item>:<path>@<logged>` for a
    /// project-input line, `<stage>/<item>:<path>#<sequence>` for a copy.
    pub landing: String,
    /// The paths it restored (or would have).
    #[serde(default)]
    pub paths: Vec<String>,
    pub fix_call_id: String,
    pub verdict_call_id: String,
    pub task_ids: Vec<String>,
    /// The refusing verdict's own summary, cut short.
    pub verdict: String,
    /// `reverted`, `already_reverted`, `conflict` or `unrevertable`.
    pub outcome: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub revert_commit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl RefusedLandingRevert {
    /// The landing is out of the tree.
    pub fn done(&self) -> bool {
        matches!(self.outcome.as_str(), "reverted" | "already_reverted")
    }
}

fn ledger_path(run_root: &Path) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("refused-landings.jsonl")
}

/// Every decision logged for this run, oldest first. A line that does not
/// parse is an error: a reader that skipped it could revert twice.
pub fn refused_landing_reverts(run_root: &Path) -> Result<Vec<RefusedLandingRevert>, String> {
    let path = ledger_path(run_root);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let whole = text.rfind('\n').map_or("", |end| &text[..=end]);
    whole
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(at, line)| {
            serde_json::from_str(line)
                .map_err(|error| format!("{} line {}: {error}", path.display(), at + 1))
        })
        .collect()
}

/// Append `line`, flushed before return.
pub(super) fn append(run_root: &Path, line: &RefusedLandingRevert) -> std::io::Result<()> {
    use std::io::Write;
    let path = ledger_path(run_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec(line).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()
}

/// The finding a write attempt at `task_ids` is given: every earlier
/// remediation of those tasks the host took back out because its verdict
/// refused it, in the verdict's own words. Empty when there is none.
pub fn refused_landings_preamble(run_root: &Path, task_ids: &[String]) -> String {
    let Ok(lines) = refused_landing_reverts(run_root) else {
        return String::new();
    };
    let mut refusals: Vec<(&str, &str, &str, Vec<&str>)> = Vec::new();
    for line in lines.iter().filter(|line| line.done()) {
        if !line.task_ids.iter().any(|task| task_ids.contains(task)) {
            continue;
        }
        let paths = line.paths.iter().map(String::as_str);
        match refusals.iter_mut().find(|(fix, verdict, _, _)| {
            *fix == line.fix_call_id && *verdict == line.verdict_call_id
        }) {
            Some((_, _, _, reverted)) => reverted.extend(paths),
            None => refusals.push((
                &line.fix_call_id,
                &line.verdict_call_id,
                &line.verdict,
                paths.collect(),
            )),
        }
    }
    if refusals.is_empty() {
        return String::new();
    }
    let skip = refusals.len().saturating_sub(PREAMBLE_REFUSALS);
    let mut text = String::from(
        "\n\n## Earlier Remediation Refused And Reverted (host)\nAn earlier attempt at these tasks landed changes that its verifier did not accept, so the host reverted them: they are NOT in the tree you start from. Treat each verifier judgment below as a finding for this attempt: address what it names, and do not land the same change again.\n",
    );
    for (fix, verdict, summary, mut paths) in refusals.into_iter().skip(skip) {
        paths.sort_unstable();
        paths.dedup();
        text.push_str(&format!(
            "- {fix}, refused by {verdict}; reverted: {}\n  verifier: {summary}\n",
            paths.join(", ")
        ));
    }
    text
}
