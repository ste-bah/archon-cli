//! Issue 391: content novelty, judged on the REAL `Read` and `Grep` tools'
//! output over real files.

use super::*;
use archon_tools::tool::{Tool, ToolContext};
use serde_json::json;

fn context(dir: &std::path::Path) -> ToolContext {
    ToolContext {
        working_dir: dir.to_path_buf(),
        ..ToolContext::default()
    }
}

/// Run `tool` for real and judge the call.
async fn judge(
    ledger: &mut ContentLedger,
    dir: &std::path::Path,
    tool: &dyn Tool,
    input: Value,
) -> Novelty {
    let result = tool.execute(input.clone(), &context(dir)).await;
    let call = ObservedCall {
        name: tool.name().to_string(),
        raw_input: input.to_string(),
        input: Some(input),
        refused: false,
        is_error: result.is_error,
        delivered: result.content,
    };
    ledger.observe(&call, &call.arguments())
}

fn write_lines(path: &std::path::Path, lines: usize, word: &str) {
    let text = (1..=lines)
        .map(|line| format!("{word} {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, text).unwrap();
}

fn read(path: &std::path::Path, offset: usize, limit: usize) -> Value {
    json!({ "file_path": path, "offset": offset, "limit": limit })
}

#[tokio::test]
async fn a_second_pass_in_new_slices_is_not_novel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    write_lines(&path, 105, "line");
    let tool = archon_tools::file_read::ReadTool;
    let mut ledger = ContentLedger::default();
    for offset in [0, 35, 70] {
        assert_eq!(
            judge(&mut ledger, dir.path(), &tool, read(&path, offset, 35)).await,
            Novelty::NewCall,
            "first pass at {offset}"
        );
    }
    for (offset, limit) in [(5, 30), (35, 30), (65, 30), (95, 30), (0, 105)] {
        assert_eq!(
            judge(&mut ledger, dir.path(), &tool, read(&path, offset, limit)).await,
            Novelty::None,
            "second pass at {offset}+{limit}"
        );
    }
    // The whole file without arguments, and with force_refresh: still seen.
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, json!({ "file_path": path })).await,
        Novelty::None
    );
    assert_eq!(
        judge(
            &mut ledger,
            dir.path(),
            &tool,
            json!({ "file_path": path, "force_refresh": true })
        )
        .await,
        Novelty::None
    );
}

#[tokio::test]
async fn an_overlap_is_novel_only_for_unseen_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    write_lines(&path, 105, "line");
    let tool = archon_tools::file_read::ReadTool;
    let mut ledger = ContentLedger::default();
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&path, 0, 35)).await,
        Novelty::NewCall
    );
    // Lines 20-55: 36-55 are new.
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&path, 19, 36)).await,
        Novelty::NewCall
    );
    // Lines 36-55 again inside a different slice: nothing new.
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&path, 35, 20)).await,
        Novelty::None
    );
}

#[tokio::test]
async fn the_same_range_of_a_changed_file_is_novel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    write_lines(&path, 40, "before");
    let tool = archon_tools::file_read::ReadTool;
    let mut ledger = ContentLedger::default();
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&path, 0, 35)).await,
        Novelty::NewCall
    );
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&path, 0, 35)).await,
        Novelty::None,
        "unchanged: the identical call is not novel"
    );
    write_lines(&path, 40, "after");
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&path, 0, 35)).await,
        Novelty::NewContent,
        "same arguments, new bytes"
    );
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&path, 10, 10)).await,
        Novelty::None,
        "the changed lines were given by the last read"
    );
}

/// A different file with the same bytes is new content: the source is the
/// file, not the text alone.
#[tokio::test]
async fn a_different_file_with_the_same_content_is_novel() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("a.md");
    let second = dir.path().join("b.md");
    write_lines(&first, 20, "line");
    write_lines(&second, 20, "line");
    let tool = archon_tools::file_read::ReadTool;
    let mut ledger = ContentLedger::default();
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&first, 0, 20)).await,
        Novelty::NewCall
    );
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&second, 0, 20)).await,
        Novelty::NewCall,
        "same text, different file"
    );
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&second, 5, 10)).await,
        Novelty::None
    );
}

#[tokio::test]
async fn a_read_error_keeps_the_argument_rule() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    write_lines(&path, 10, "line");
    let missing = dir.path().join("b.md");
    let tool = archon_tools::file_read::ReadTool;
    let mut ledger = ContentLedger::default();
    let missing_read = json!({ "file_path": missing });
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, missing_read.clone()).await,
        Novelty::NewCall
    );
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, missing_read).await,
        Novelty::None,
        "the same missing path again"
    );
    // Past the end of the file: an error, judged by its arguments.
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, read(&path, 50, 10)).await,
        Novelty::NewCall
    );
}

#[tokio::test]
async fn a_note_appended_to_a_read_result_is_not_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    write_lines(&path, 20, "line");
    let tool = archon_tools::file_read::ReadTool;
    let mut ledger = ContentLedger::default();
    judge(&mut ledger, dir.path(), &tool, read(&path, 0, 20)).await;
    // The soft inspection ceiling appends a counted note to the result.
    let result = tool.execute(read(&path, 5, 10), &context(dir.path())).await;
    let input = read(&path, 5, 10);
    let call = ObservedCall {
        name: "Read".into(),
        raw_input: input.to_string(),
        input: Some(input),
        refused: false,
        is_error: false,
        delivered: format!(
            "{}\n\nYou have made 91 inspection calls; produce your deliverable now.",
            result.content
        ),
    };
    assert_eq!(ledger.observe(&call, &call.arguments()), Novelty::None);
}

#[tokio::test]
async fn grep_is_novel_on_a_new_search_and_not_on_a_reslice() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
    std::fs::write(dir.path().join("two.rs"), "fn gamma() {}\n").unwrap();
    let tool = archon_tools::grep::GrepTool;
    let mut ledger = ContentLedger::default();
    let search = |path: &str, extra: Value| {
        let mut input = json!({
            "pattern": "fn ",
            "path": dir.path().join(path),
            "output_mode": "content",
        });
        if let (Some(input), Some(extra)) = (input.as_object_mut(), extra.as_object()) {
            input.extend(extra.clone());
        }
        input
    };
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, search("one.rs", json!({}))).await,
        Novelty::NewCall
    );
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, search("one.rs", json!({}))).await,
        Novelty::None,
        "a search already answered"
    );
    assert_eq!(
        judge(&mut ledger, dir.path(), &tool, search("two.rs", json!({}))).await,
        Novelty::NewCall,
        "the same pattern on a different path"
    );
    // The same search re-sliced: new arguments, no new line.
    for head_limit in ["40", "6"] {
        assert_eq!(
            judge(
                &mut ledger,
                dir.path(),
                &tool,
                search("one.rs", json!({ "head_limit": head_limit }))
            )
            .await,
            Novelty::None,
            "head_limit {head_limit}"
        );
    }
}

#[test]
fn own_evidence_is_never_novel() {
    let mut ledger = ContentLedger::default();
    for (offset, char_offset) in [(0, 0), (74, 29_592), (74, 62_360), (75, 0)] {
        let input = json!({ "offset": offset, "limit": 10, "char_offset": char_offset });
        let call = ObservedCall {
            name: "read-own-evidence".into(),
            raw_input: input.to_string(),
            input: Some(input),
            refused: false,
            is_error: false,
            delivered: format!(r#"{{"offset":{offset},"lines":["chunk {char_offset}"]}}"#),
        };
        assert_eq!(
            ledger.observe(&call, &call.arguments()),
            Novelty::None,
            "the session's own transcript holds nothing it was not given"
        );
    }
}

#[test]
fn a_call_without_paging_arguments_keeps_the_argument_rule() {
    let mut ledger = ContentLedger::default();
    let call = |command: &str| {
        let input = json!({ "command": command });
        ObservedCall {
            name: "Bash".into(),
            raw_input: input.to_string(),
            input: Some(input),
            refused: false,
            is_error: false,
            delivered: "same output".into(),
        }
    };
    let first = call("ls src");
    assert_eq!(ledger.observe(&first, &first.arguments()), Novelty::NewCall);
    let second = call("ls tests");
    assert_eq!(
        ledger.observe(&second, &second.arguments()),
        Novelty::NewCall,
        "new arguments, same text: unchanged behaviour"
    );
    assert_eq!(ledger.observe(&first, &first.arguments()), Novelty::None);
}

/// Issue 391 evidence: replay a recorded subagent transcript (path in
/// `ARCHON_NOVELTY_REPLAY`) through the ledger and through the argument rule
/// it replaced, and print how many admitted calls each counts as novel.
/// Ignored: it reads a file outside the repository.
#[test]
#[ignore = "evidence replay; needs ARCHON_NOVELTY_REPLAY=<transcript.jsonl>"]
fn replay_a_recorded_transcript() {
    let path = std::env::var("ARCHON_NOVELTY_REPLAY").expect("ARCHON_NOVELTY_REPLAY");
    let text = std::fs::read_to_string(&path).expect("read the transcript");
    let mut pending: std::collections::HashMap<String, (String, Value)> = Default::default();
    let mut ledger = ContentLedger::default();
    let mut argument_rule = std::collections::HashSet::new();
    let (mut calls, mut old_novel, mut new_novel, mut refused) = (0, 0, 0, 0);
    // Longest run of admitted calls that renewed nothing, under each rule.
    let (mut old_run, mut new_run, mut old_longest, mut new_longest) = (0, 0, 0, 0);
    let mut by_tool: std::collections::BTreeMap<String, (u32, u32, u32)> = Default::default();
    let mut last_new = String::new();
    let mut last_old = String::new();
    for line in text.lines() {
        let message: Value = serde_json::from_str(line).expect("transcript line");
        let Some(blocks) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    pending.insert(
                        block["id"].as_str().unwrap_or_default().to_string(),
                        (
                            block["name"].as_str().unwrap_or_default().to_string(),
                            block["input"].clone(),
                        ),
                    );
                }
                Some("tool_result") => {
                    let Some((name, input)) = block["tool_use_id"]
                        .as_str()
                        .and_then(|id| pending.remove(id))
                    else {
                        continue;
                    };
                    let delivered = match &block["content"] {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    let is_error = block["is_error"].as_bool().unwrap_or(false);
                    calls += 1;
                    if is_error
                        && (delivered.contains("read ceiling reached:")
                            || delivered.contains("inspection withdrawn:"))
                    {
                        refused += 1;
                        continue;
                    }
                    let call = ObservedCall {
                        name: name.clone(),
                        raw_input: input.to_string(),
                        input: Some(input),
                        refused: false,
                        is_error,
                        delivered,
                    };
                    let arguments = call.arguments();
                    let entry = by_tool.entry(name.clone()).or_default();
                    entry.0 += 1;
                    if argument_rule.insert((name.clone(), arguments.clone())) {
                        old_novel += 1;
                        entry.1 += 1;
                        last_old = format!("call {calls}: {name} {arguments}");
                        old_run = 0;
                    } else {
                        old_run += 1;
                        old_longest = old_longest.max(old_run);
                    }
                    if ledger.observe(&call, &arguments) != Novelty::None {
                        new_novel += 1;
                        entry.2 += 1;
                        last_new = format!("call {calls}: {name} {arguments}");
                        new_run = 0;
                    } else {
                        new_run += 1;
                        new_longest = new_longest.max(new_run);
                    }
                }
                _ => {}
            }
        }
    }
    println!("REPLAY {path}");
    println!(
        "REPLAY calls={calls} refused={refused} novel_by_arguments={old_novel} novel_by_content={new_novel}"
    );
    println!("REPLAY longest_non_novel_run by_arguments={old_longest} by_content={new_longest}");
    for (tool, (count, old, new)) in by_tool {
        println!(
            "REPLAY tool={tool} calls={count} novel_by_arguments={old} novel_by_content={new}"
        );
    }
    println!(
        "REPLAY last_novel_by_arguments: {}",
        last_old.chars().take(240).collect::<String>()
    );
    println!(
        "REPLAY last_novel_by_content: {}",
        last_new.chars().take(240).collect::<String>()
    );
}
