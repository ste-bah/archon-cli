//! R17 changes only the existing script's fixture content and selected replies.
use super::super::script::{body_author_marker, judge_decision, scripted_answer, user_prompt_text};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    V0,
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8,
}

pub fn attempt(prompt: &str) -> usize {
    user_prompt_text(prompt)
        .unwrap_or_default()
        .rsplit_once("authored call ")
        .and_then(|(_, s)| s.split('.').next()?.parse().ok())
        .unwrap_or(1)
}

pub fn skeleton(prompt: &str, cell: Cell) -> Value {
    // Use the existing candidate builder, including its current product marker.
    let (_, base) = scripted_answer(prompt);
    let mut candidate: Value = serde_json::from_str(&base).unwrap();
    candidate["tasks"][0]["source_sections"] = json!(["0", "1", "2", "3.1", "4", "5", "8", "9"]);
    candidate["tasks"][1]["source_sections"] = json!(["2", "3.2", "6"]);
    candidate["tasks"][0]["implements"] = json!(["REQ-X-001", "AC-X-001", "C-X-001", "C-X-003"]);
    candidate["tasks"][1]["implements"] =
        json!(["REQ-X-002", "REQ-X-003", "C-X-001", "C-X-002", "C-X-003"]);
    if matches!(cell, Cell::V1 | Cell::V8) && (cell == Cell::V8 || attempt(prompt) == 1) {
        candidate["tasks"][1]["implements"] =
            json!(["REQ-X-002", "REQ-X-003", "C-X-001", "C-X-003"]);
        candidate["tasks"][0]["source_sections"] = json!(["0", "1", "2", "3.1", "4", "5", "8"]);
        if cell == Cell::V1 {
            candidate["tasks"][0]["source_sections"]
                .as_array_mut()
                .unwrap()
                .push(json!("2"));
        }
    }
    if cell == Cell::V3 {
        let contract = &mut candidate["tasks"][0]["deliverable_contracts"][0];
        contract["instance_source_records_field"] = json!("items");
        contract["registry_records_field"] = json!("entries");
        let consume = &mut candidate["tasks"][1]["depends_on"][0]["consumes"][0];
        consume["instance_source_records_field"] = json!("items");
        consume["registry_records_field"] = json!("entries");
    }
    if cell == Cell::V5 && attempt(prompt) > 1 {
        candidate["tasks"][1]["source_sections"] = json!(["1", "2", "3.2", "6"]);
        // Patch only the named task; the host seeds the first task's record.
        candidate["tasks"] = json!([candidate["tasks"][1].clone()]);
    }
    candidate
}

pub fn answer(prompt: &str, cell: Cell) -> (&'static str, String) {
    let text = user_prompt_text(prompt).unwrap_or_default();
    if text.contains("Adversarially judge every acceptance check below.")
        || text.starts_with("Each acceptance check below already ran")
    {
        let checks: Vec<Value> = text
            .rsplit_once("Checks: ")
            .and_then(|(_, s)| serde_json::from_str(s).ok())
            .unwrap();
        return ("judge", json!({"decisions":checks.iter().map(|c| judge_decision(c["id"].as_str().unwrap(), false)).collect::<Vec<_>>()} ).to_string());
    }
    if text.contains("Author ONLY entry SUP-REQ-X-003:") {
        return ("acceptance", json!({"id":"SUP-REQ-X-003", "criterion":"The second artifact remains valid JSON.",
            "check":{"kind":"command", "command":"python3 -c 'import json; json.load(open(\"src/beta.json\"))'", "cwd":"repo_root"},
            "gap_permitted":false, "covers":["REQ-X-003"],
            "judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}).to_string());
    }
    let (role, mut reply) = scripted_answer(prompt);
    match role {
        "skeleton" => reply = skeleton(prompt, cell).to_string(),
        "body" => {
            let second = text.contains(&format!("{}TASK-X-020.", body_author_marker()));
            let claims = if second {
                "C-X-001 and C-X-002 and C-X-003 and REQ-X-003 are verified by `python3 -m json.tool src/beta.json`; preserve src/alpha.txt unchanged."
            } else {
                "C-X-001 and C-X-003 are verified by `test -s src/alpha.txt`; preserve its text format."
            };
            reply.push_str(&format!("\n{claims}\n"));
            // Frozen metadata is stamped by the host; name each id only in yaml
            // on the first faulty body, then repair its prose on the next call.
            if cell == Cell::V2 && !second && attempt(prompt) == 1 {
                reply = reply.replace("C-X-001 and ", "");
                reply = reply.replacen("implements: [", "implements: [C-X-001, ", 1);
            }
            if cell == Cell::V6 && !second && attempt(prompt) == 1 {
                reply = reply.replacen("\n```\n", "\n", 1);
            }
        }
        _ => {}
    }
    (role, reply)
}

/// Fault only the first full-set critic request; claimant-cluster/body
/// requests keep the script's answer. The server supplies its request history.
pub fn critic(
    prompt: &str,
    reply: String,
    cell: Cell,
    previous: &[super::super::RequestRecord],
) -> String {
    let text = user_prompt_text(prompt).unwrap_or_default();
    let full_set = text.contains("===== BEGIN TASK TASK-X-010 =====")
        && text.contains("===== BEGIN TASK TASK-X-020 =====");
    if !full_set {
        return reply;
    }
    // The baseline has two body critic calls; the third belongs to the set.
    if previous.iter().filter(|r| r.role == "fidelity").count() < 2 {
        return reply;
    }
    if cell == Cell::V4
        && serde_json::from_str::<Value>(prompt).ok().is_some_and(|r| {
            r["messages"].as_array().is_some_and(|messages| {
                messages
                    .iter()
                    .any(|m| m["role"] == "assistant" && m["content"] == "{\"verdicts\":[")
            })
        })
    {
        // The output-limit continuation cannot extend the truncated reply.
        // This ends the critic call as authored_judge_reply, not a re-ask
        // for malformed end_turn JSON inside the same gate invocation.
        return String::new();
    }
    let already = previous.iter().any(|r| {
        r.role == "fidelity"
            && (r.answer == "{\"verdicts\":["
                || r.answer.contains("R17 named-task section ownership"))
    });
    if already {
        return reply;
    }
    if cell == Cell::V4 {
        return "{\"verdicts\":[".into();
    }
    if cell == Cell::V5 {
        let mut response: Value = serde_json::from_str(&reply).unwrap();
        let verdict = &mut response["verdicts"][0];
        verdict["necessarily_true"] = json!(false);
        verdict["weakest_task_id"] = json!("TASK-X-020");
        verdict["reason"] =
            json!("R17 named-task section ownership: task TASK-X-020 must also cite section 1.");
        verdict["quoted_task_text"] = json!(
            text.split_once("===== BEGIN TASK TASK-X-020 =====")
                .unwrap()
                .1
                .split_once("## Objective\n")
                .unwrap()
                .1
                .lines()
                .next()
                .unwrap()
        );
        verdict["weakening_quotes"] = json!([]);
        verdict["supporting_task_id"] = json!("");
        verdict["supporting_text"] = json!("");
        return response.to_string();
    }
    reply
}
