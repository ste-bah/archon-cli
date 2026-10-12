//! Fault-cell variants of the scripted replies, and the per-call key every
//! provider request is counted under.
//!
//! A variant changes one reply of the golden script and nothing else, so a
//! cell's result differs from golden only where that reply reaches.

use super::script::{scripted_answer, user_prompt_text};

#[path = "workflow_decomposition_offline_server_large.rs"]
pub mod large;

/// A secret-shaped token (an Anthropic key prefix). Redaction of public
/// copies replaces such a word with `<redacted>`.
pub const SECRET_TOKEN: &str = "sk-ant-api03-offlineFixtureSecret0123456789";

/// The CRLF verifier the `CrlfCheck` variant authors for SUP-REQ-X-001.
pub const CRLF_SCRIPT: &str =
    "set -eu\r\ntest -f src/alpha.txt\r\ngrep -qx 'alpha ready' src/alpha.txt\r\n";
/// The heredoc verifier the `HeredocCheck` variant authors for SUP-REQ-X-002.
pub const HEREDOC_SCRIPT: &str = "set -eu\npython3 - src/beta.json <<'PY'\nimport json, sys\nassert json.load(open(sys.argv[1]))[\"ready\"] is True\nPY\n";

// The script cells that build the CRLF and heredoc variants run on Unix.
#[cfg_attr(not(unix), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Golden,
    /// The AC-X-001 author reply carries a secret-shaped word after its entry.
    SecretInAcceptanceReply,
    /// SUP-REQ-X-001 is authored as a check block with CRLF line ends.
    CrlfCheck,
    /// SUP-REQ-X-002 is authored as a check block holding a heredoc.
    HeredocCheck,
    /// The PRD has this many acceptance criteria (`large::prd`).
    Large(usize),
}

pub fn answer(prompt: &str, variant: Variant) -> (&'static str, String) {
    if let Variant::Large(n) = variant {
        return large::answer(prompt, n);
    }
    let (role, answer) = scripted_answer(prompt);
    if role != "acceptance" {
        return (role, answer);
    }
    let entry: serde_json::Value = serde_json::from_str(&answer).unwrap();
    let id = entry["id"].as_str().unwrap_or_default();
    match (variant, id) {
        (Variant::SecretInAcceptanceReply, "AC-X-001") => (
            role,
            format!("```json\n{answer}\n```\nUnused deployment credential: {SECRET_TOKEN}\n"),
        ),
        (Variant::CrlfCheck, "SUP-REQ-X-001") => (role, check_block_reply(entry, CRLF_SCRIPT)),
        (Variant::HeredocCheck, "SUP-REQ-X-002") => {
            (role, check_block_reply(entry, HEREDOC_SCRIPT))
        }
        _ => (role, answer),
    }
}

/// The reply format the acceptance prompt asks for when a check is a script:
/// the entry with `command_block: true`, then one ```check fence.
fn check_block_reply(mut entry: serde_json::Value, script: &str) -> String {
    entry["check"] =
        serde_json::json!({"kind": "command", "command_block": true, "cwd": "repo_root"});
    let newline = if script.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    format!("```json{newline}{entry}{newline}```{newline}```check{newline}{script}```{newline}")
}

/// The call a provider request belongs to: its role and the ids it is about.
/// A resumed run that re-asks an accepted call shows up as a higher count.
pub fn request_key(role: &str, prompt: &str) -> String {
    let text = user_prompt_text(prompt).unwrap_or_default();
    let after = |marker: &str, end: char| {
        text.split_once(marker)
            .and_then(|(_, rest)| rest.split_once(end).map(|(id, _)| id.trim().to_string()))
            .unwrap_or_default()
    };
    match role {
        "acceptance" => format!("acceptance:{}", after("Author ONLY entry ", ':')),
        "body" => format!(
            "body:{}",
            after(
                "Author the complete TASK body for host-frozen task_id ",
                '.'
            )
        ),
        "judge" => {
            let kind = if text.starts_with("Each acceptance check below already ran once") {
                "passability"
            } else {
                "adversarial"
            };
            let ids = text
                .rsplit_once("Checks: ")
                .and_then(|(_, json)| serde_json::from_str::<Vec<serde_json::Value>>(json).ok())
                .unwrap_or_default();
            format!("judge:{kind}:{}", ids_of(&ids, "id"))
        }
        "fidelity" => {
            let obligations = text
                .split_once("Obligations: ")
                .and_then(|(_, rest)| rest.split_once("\n\n"))
                .and_then(|(json, _)| serde_json::from_str::<Vec<serde_json::Value>>(json).ok())
                .unwrap_or_default();
            // The body gate shows the claiming task; the set gate every task.
            let tasks = ["TASK-X-010", "TASK-X-020"]
                .into_iter()
                .filter(|task| text.contains(&format!("task_id: {task}")))
                .collect::<Vec<_>>()
                .join("+");
            format!("fidelity:{}:{tasks}", ids_of(&obligations, "id"))
        }
        other => other.to_string(),
    }
}

fn ids_of(items: &[serde_json::Value], field: &str) -> String {
    let mut ids = items
        .iter()
        .filter_map(|item| item.get(field).and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.join(",")
}
