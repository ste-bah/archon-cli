//! Issue 391 on the REAL runner path: novelty is new CONTENT, not a new
//! (path, offset) tuple. These run the real `Read` and `read-own-evidence`
//! tools over real files, with the scripted provider and window of the
//! parent module.

use super::*;

/// A runner whose only tool is `tool`, working in `dir`.
fn runner_with_tool(
    provider: Arc<ScriptedProvider>,
    session: &str,
    dir: &std::path::Path,
    tool: Box<dyn Tool>,
) -> SubagentRunner {
    let mut registry = crate::dispatch::ToolRegistry::new();
    registry.register(tool);
    let definitions = registry.tool_definitions();
    let context = ToolContext {
        working_dir: dir.to_path_buf(),
        session_id: session.into(),
        mode: archon_tools::tool::AgentMode::Normal,
        ..Default::default()
    };
    SubagentRunner::new(
        provider,
        "You are a test subagent.".into(),
        definitions,
        Arc::new(registry),
        context,
        "mock-model".into(),
        10_000,
        300,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    )
}

async fn run_with_tool(
    turns: Vec<Turn>,
    session: &str,
    dir: &std::path::Path,
    name: &'static str,
    tool: Box<dyn Tool>,
) -> (anyhow::Result<String>, u64) {
    let clock = DispatchClock::new();
    clock.admit();
    let runner = runner_with_tool(ScriptedProvider::calling(turns, name), session, dir, tool);
    let started = tokio::time::Instant::now();
    let result = scope_session(
        session,
        vec![Arc::clone(&clock)],
        scope(HostTimeout::Finite(WINDOW), runner.run("go")),
    )
    .await;
    (result, started.elapsed().as_secs())
}

/// A real file of `lines` distinct lines.
fn numbered_file(dir: &std::path::Path, lines: usize) -> std::path::PathBuf {
    let path = dir.join("context.md");
    let text = (1..=lines)
        .map(|line| format!("line {line} of the author context"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, text).unwrap();
    path
}

fn slice(path: &std::path::Path, offset: usize, limit: usize) -> Option<String> {
    Some(serde_json::json!({ "file_path": path, "offset": offset, "limit": limit }).to_string())
}

#[tokio::test(start_paused = true)]
async fn paging_a_file_twice_renews_the_window_only_on_the_first_pass() {
    let dir = tempfile::tempdir().unwrap();
    let path = numbered_file(dir.path(), 105);
    // Pass 1: three 35-line slices cover the whole file. Pass 2: the same
    // file again in 30-line slices at new offsets — new arguments, no new
    // content. The last turn repeats for as long as the session asks.
    let mut turns: Vec<Turn> = [0, 35, 70]
        .into_iter()
        .map(|offset| (None, slice(&path, offset, 35)))
        .collect();
    turns.extend(
        [5, 35, 65, 95]
            .into_iter()
            .map(|offset| (None, slice(&path, offset, 30))),
    );
    let (result, elapsed) = run_with_tool(
        turns,
        "pw-391-page-twice",
        dir.path(),
        "Read",
        Box::new(archon_tools::file_read::ReadTool),
    )
    .await;
    let text = stall_text(result);
    assert!(
        text.contains("last novel activity: turn 3: new tool call Read"),
        "{text}"
    );
    assert_eq!(elapsed, 3 * 500 + WINDOW, "the second pass renewed: {text}");
}

#[tokio::test(start_paused = true)]
async fn an_overlapping_slice_is_novel_only_for_its_unseen_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = numbered_file(dir.path(), 105);
    let turns: Vec<Turn> = vec![
        (None, slice(&path, 0, 35)),  // lines 1-35: new
        (None, slice(&path, 19, 36)), // lines 20-55: 36-55 new
        (None, slice(&path, 10, 20)), // lines 11-30: all seen
        (None, slice(&path, 40, 10)), // lines 41-50: all seen
    ];
    let (result, elapsed) = run_with_tool(
        turns,
        "pw-391-overlap",
        dir.path(),
        "Read",
        Box::new(archon_tools::file_read::ReadTool),
    )
    .await;
    let text = stall_text(result);
    assert!(
        text.contains("last novel activity: turn 2: new tool call Read"),
        "{text}"
    );
    assert!(text.contains("\"offset\":19"), "{text}");
    assert_eq!(elapsed, 2 * 500 + WINDOW, "{text}");
}

#[tokio::test(start_paused = true)]
async fn reading_own_evidence_back_never_renews_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let evidence = dir.path().join("own.jsonl");
    // One 100 000-char line: read-own-evidence returns it in 32 768-byte
    // chunks, so a model pages it with char_offset, each chunk a new slice
    // of a message this session already received.
    let line: String = (0..100_000)
        .map(|index| char::from(b'a' + (index % 26) as u8))
        .collect();
    std::fs::write(&evidence, format!("{line}\n")).unwrap();
    let chunk = |char_offset: usize| {
        Some(serde_json::json!({ "offset": 0, "limit": 1, "char_offset": char_offset }).to_string())
    };
    let turns: Vec<Turn> = [0, 32_768, 65_536, 98_304, 1_000, 16_384]
        .into_iter()
        .map(|char_offset| (None, chunk(char_offset)))
        .collect();
    let (result, elapsed) = run_with_tool(
        turns,
        "pw-391-own-evidence",
        dir.path(),
        "read-own-evidence",
        Box::new(crate::subagent::runner::evidence_recovery::ReadOwnEvidence(
            evidence,
        )),
    )
    .await;
    let text = stall_text(result);
    assert!(!text.contains("new tool call read-own-evidence"), "{text}");
    assert_eq!(elapsed, WINDOW, "no own-evidence slice renewed: {text}");
}

/// Issue 391 negative control: genuinely new slices keep renewing, so a
/// session reading a large file once, slice by slice, is never cut.
#[tokio::test(start_paused = true)]
async fn reading_new_slices_of_one_file_keeps_renewing() {
    let dir = tempfile::tempdir().unwrap();
    let path = numbered_file(dir.path(), 30 * 35);
    let mut turns: Vec<Turn> = (0..30)
        .map(|step| (None, slice(&path, step * 35, 35)))
        .collect();
    turns.push((Some("done"), None));
    let (result, elapsed) = run_with_tool(
        turns,
        "pw-391-new-slices",
        dir.path(),
        "Read",
        Box::new(archon_tools::file_read::ReadTool),
    )
    .await;
    assert_eq!(result.expect("new content is progress"), "done");
    assert!(elapsed >= 15_000, "ran {elapsed}s, past 7200s in total");
}
