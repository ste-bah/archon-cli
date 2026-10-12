//! The large-candidate variant: the fixture PRD with `n` acceptance
//! criteria. AC-X-001 is the golden criterion; AC-X-002.. name one more
//! boolean field of the same outcome artifact each, so every check is
//! distinct, executable and owned by the first task.

use super::super::script::{scripted_answer, user_prompt_text};

fn field(k: usize) -> String {
    if k == 1 {
        "ready".into()
    } else {
        format!("ready_{k:03}")
    }
}

fn ac_id(k: usize) -> String {
    format!("AC-X-{k:03}")
}

fn criterion(k: usize) -> String {
    format!(
        "The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `{}` set to true.",
        field(k)
    )
}

fn check(k: usize) -> String {
    format!(
        "python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"{}\"] is True' .archon/proof/outcome.json",
        field(k)
    )
}

/// The PRD: the fixture's, with acceptance rows AC-X-001..`n`.
pub fn prd(base: &str, n: usize) -> String {
    let mut prd = base.trim_end().to_string();
    for k in 2..=n {
        prd.push_str(&format!("\n| {} | {} |", ac_id(k), criterion(k)));
    }
    prd.push('\n');
    prd
}

/// The outcome artifact every check reads, with every field true.
pub fn outcome(n: usize) -> String {
    let fields = (1..=n)
        .map(|k| (field(k), serde_json::Value::Bool(true)))
        .collect::<serde_json::Map<_, _>>();
    serde_json::Value::Object(fields).to_string()
}

fn known_id(id: &str, n: usize) -> bool {
    matches!(id, "SUP-REQ-X-001" | "SUP-REQ-X-002") || (1..=n).any(|k| id == ac_id(k))
}

pub fn answer(prompt: &str, n: usize) -> (&'static str, String) {
    let text = user_prompt_text(prompt).unwrap_or_default();
    if text.contains("Author exactly one acceptance entry identified below")
        && let Some(id) = text
            .split_once("Author ONLY entry ")
            .and_then(|(_, rest)| rest.split_once(':').map(|(id, _)| id.trim().to_string()))
        && let Some(k) = (2..=n).find(|k| id == ac_id(*k))
    {
        let authored = text
            .split_once(&format!("Author ONLY entry {id}:"))
            .map(|(_, rest)| rest.trim_start())
            .unwrap_or_default();
        if !authored.starts_with(&criterion(k)) {
            return unknown();
        }
        return (
            "acceptance",
            serde_json::json!({
                "id": id,
                "criterion": criterion(k),
                "check": {"kind": "command", "command": check(k), "cwd": "repo_root"},
                "gap_permitted": false,
                "covers": [],
                "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
            })
            .to_string(),
        );
    }
    if text.contains("Adversarially judge every acceptance check below.")
        || text.starts_with("Each acceptance check below already ran once")
    {
        let Some(checks) = text
            .rsplit_once("Checks: ")
            .and_then(|(_, json)| serde_json::from_str::<Vec<serde_json::Value>>(json).ok())
        else {
            return unknown();
        };
        let ids = checks
            .iter()
            .filter_map(|check| check.get("id").and_then(serde_json::Value::as_str))
            .collect::<std::collections::BTreeSet<_>>();
        if ids.is_empty() || ids.iter().any(|id| !known_id(id, n)) {
            return unknown();
        }
        let decisions = ids
            .into_iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "verdict": "accepted",
                    "counterexample": "None found: a missing or false outcome makes the command exit non-zero.",
                    "reason": "The command directly checks the declared outcome and exits non-zero when it is false."
                })
            })
            .collect::<Vec<_>>();
        return (
            "judge",
            serde_json::json!({"decisions": decisions}).to_string(),
        );
    }
    let (role, answer) = scripted_answer(prompt);
    let extra_ids = (2..=n).map(ac_id).collect::<Vec<_>>();
    match role {
        "skeleton" => {
            let mut skeleton: serde_json::Value = serde_json::from_str(&answer).unwrap();
            let implements = skeleton["tasks"][0]["implements"].as_array_mut().unwrap();
            implements.extend(extra_ids.iter().map(|id| serde_json::json!(id)));
            (role, skeleton.to_string())
        }
        "body" if answer.contains("task_id: TASK-X-010") => {
            let claims = (2..=n)
                .map(|k| format!(" {} is verified by `{}`.", ac_id(k), check(k)))
                .collect::<String>();
            let tests = (2..=n)
                .map(|k| format!("- `{}`\n", check(k)))
                .collect::<String>();
            let body = answer
                .replace(
                    "implements: [REQ-X-001, AC-X-001]",
                    &format!(
                        "implements: [REQ-X-001, AC-X-001, {}]",
                        extra_ids.join(", ")
                    ),
                )
                .replace(
                    "\n\n## Files Expected to Change",
                    &format!("{claims}\n\n## Files Expected to Change"),
                );
            (role, format!("{body}{tests}"))
        }
        _ => (role, answer),
    }
}

fn unknown() -> (&'static str, String) {
    (
        "unknown",
        r#"{"error":"unscripted fake provider request"}"#.into(),
    )
}
