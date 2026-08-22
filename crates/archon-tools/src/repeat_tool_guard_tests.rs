//! Behaviour of the repeat-tool chain (#200 Phase 2).
//!
//! Every test uses its own registry rather than [`REPEAT_TOOL_CHAINS`], so the
//! process-global one is never a shared fixture between parallel tests.

use super::*;

fn config() -> RepeatToolConfig {
    RepeatToolConfig::default()
}

fn grep(pattern: &str) -> serde_json::Value {
    serde_json::json!({"pattern": pattern, "path": "crates"})
}

/// Observe `times` identical calls and return the reminders each one produced,
/// draining after every call the way both tool loops do.
fn run(
    chains: &RepeatToolChains,
    key: &ChainKey,
    config: &RepeatToolConfig,
    tool: &str,
    input: &serde_json::Value,
    times: usize,
) -> Vec<Vec<String>> {
    (0..times)
        .map(|_| {
            chains.observe(key, config, tool, input, false);
            chains.take_reminders(key)
        })
        .collect()
}

#[test]
fn a_run_produces_one_reminder_at_each_configured_length_and_none_between() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-thresholds", None);

    let per_call = run(&chains, &key, &config(), "Grep", &grep("needle"), 9);

    let fired: Vec<usize> = per_call
        .iter()
        .enumerate()
        .filter(|(_, reminders)| !reminders.is_empty())
        .map(|(index, _)| index + 1)
        .collect();
    assert_eq!(
        fired,
        vec![3, 5, 8],
        "reminders must land exactly on the configured run lengths"
    );
    assert!(per_call.iter().all(|reminders| reminders.len() <= 1));
    assert!(per_call[2][0].contains("called Grep 3 times in a row"));
    assert!(per_call[4][0].contains("called Grep 5 times in a row"));
}

/// The escalation is the point of an ordered threshold list: the same run gets
/// a nudge, then instructions, then a flat instruction to stop.
#[test]
fn each_reminder_in_a_run_is_more_emphatic_than_the_last() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-escalation", None);

    let per_call = run(&chains, &key, &config(), "Grep", &grep("needle"), 8);

    let first = &per_call[2][0];
    let second = &per_call[4][0];
    let third = &per_call[7][0];
    assert!(first.contains("change something material"));
    assert!(second.contains("Re-read the last result of this call"));
    assert!(third.contains("This is a loop and it will not break itself"));
}

/// The semantic a bookkeeping tool would otherwise launder: `TodoWrite` between
/// two identical `Grep`s leaves a run of two, not two runs of one.
#[test]
fn an_excluded_tool_between_identical_calls_neither_extends_nor_breaks_the_run() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-excluded", None);
    let config = config();
    let needle = grep("needle");

    chains.observe(&key, &config, "Grep", &needle, false);
    chains.observe(
        &key,
        &config,
        "TodoWrite",
        &serde_json::json!({"todos": []}),
        false,
    );
    assert_eq!(
        chains.run_length(&key),
        1,
        "an excluded tool must not extend the run"
    );
    chains.observe(&key, &config, "Grep", &needle, false);
    assert_eq!(
        chains.run_length(&key),
        2,
        "an excluded tool must not reset the run"
    );

    chains.observe(
        &key,
        &config,
        "TodoWrite",
        &serde_json::json!({"todos": []}),
        false,
    );
    chains.observe(&key, &config, "Grep", &needle, false);
    assert_eq!(
        chains.take_reminders(&key).len(),
        1,
        "the third Grep completes the run despite the interleaved TodoWrite"
    );
}

#[test]
fn a_different_tool_breaks_the_run() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-other-tool", None);
    let config = config();
    let needle = grep("needle");

    chains.observe(&key, &config, "Grep", &needle, false);
    chains.observe(&key, &config, "Grep", &needle, false);
    chains.observe(
        &key,
        &config,
        "Read",
        &serde_json::json!({"file_path": "a"}),
        false,
    );
    chains.observe(&key, &config, "Grep", &needle, false);

    assert_eq!(chains.run_length(&key), 1);
    assert!(chains.take_reminders(&key).is_empty());
}

#[test]
fn different_arguments_break_the_run() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-other-args", None);
    let config = config();

    chains.observe(&key, &config, "Grep", &grep("one"), false);
    chains.observe(&key, &config, "Grep", &grep("two"), false);
    chains.observe(&key, &config, "Grep", &grep("three"), false);

    assert_eq!(chains.run_length(&key), 1);
    assert!(chains.take_reminders(&key).is_empty());
}

/// Two spellings of the same call are the same call.
#[test]
fn argument_key_order_does_not_start_a_new_run() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-key-order", None);
    let config = config();

    chains.observe(
        &key,
        &config,
        "Grep",
        &serde_json::json!({"a": 1, "b": {"x": 1, "y": 2}}),
        false,
    );
    chains.observe(
        &key,
        &config,
        "Grep",
        &serde_json::json!({"b": {"y": 2, "x": 1}, "a": 1}),
        false,
    );

    assert_eq!(chains.run_length(&key), 2);
}

/// Array order, unlike object key order, is meaning — reordering it is a
/// different call and must break the run.
#[test]
fn array_order_is_part_of_the_call() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-array-order", None);
    let config = config();

    chains.observe(
        &key,
        &config,
        "Grep",
        &serde_json::json!({"paths": ["a", "b"]}),
        false,
    );
    chains.observe(
        &key,
        &config,
        "Grep",
        &serde_json::json!({"paths": ["b", "a"]}),
        false,
    );

    assert_eq!(chains.run_length(&key), 1);
}

/// A model hammering a call permissions keep rejecting is the loop most worth
/// breaking, and the advisory has to say why the result never changes.
#[test]
fn refused_calls_count_and_the_reminder_names_the_refusals() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-refused", None);
    let config = config();
    let command = serde_json::json!({"command": "rm -rf /"});

    chains.observe(&key, &config, "Bash", &command, true);
    chains.observe(&key, &config, "Bash", &command, true);
    chains.observe(&key, &config, "Bash", &command, true);

    let reminders = chains.take_reminders(&key);
    assert_eq!(reminders.len(), 1);
    assert!(reminders[0].contains("called Bash 3 times in a row"));
    assert!(
        reminders[0].contains("Every one of those 3 calls was refused"),
        "got: {}",
        reminders[0]
    );
}

#[test]
fn a_partly_refused_run_reports_how_many_were_refused() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-partly-refused", None);
    let config = config();
    let command = serde_json::json!({"command": "ls"});

    chains.observe(&key, &config, "Bash", &command, true);
    chains.observe(&key, &config, "Bash", &command, false);
    chains.observe(&key, &config, "Bash", &command, false);

    let reminders = chains.take_reminders(&key);
    assert!(
        reminders[0].contains("1 of those 3 calls were refused"),
        "got: {}",
        reminders[0]
    );
}

/// One agent's repetition must never trip another's counter, and
/// `session_id` alone cannot tell them apart because it is copied verbatim
/// from parent to child.
#[test]
fn a_subagents_run_is_not_its_parents_run() {
    let chains = RepeatToolChains::default();
    let config = config();
    let parent = ChainKey::new("shared-session", None);
    let child = ChainKey::new("shared-session", Some("child-1"));
    let needle = grep("needle");

    for _ in 0..3 {
        chains.observe(&child, &config, "Grep", &needle, false);
    }

    assert_eq!(chains.run_length(&parent), 0);
    assert!(
        chains.take_reminders(&parent).is_empty(),
        "the child's loop must not be delivered to the parent"
    );
    assert_eq!(chains.take_reminders(&child).len(), 1);
}

#[test]
fn two_subagents_of_one_session_do_not_share_a_run() {
    let chains = RepeatToolChains::default();
    let config = config();
    let first = ChainKey::new("shared-session", Some("child-1"));
    let second = ChainKey::new("shared-session", Some("child-2"));
    let needle = grep("needle");

    chains.observe(&first, &config, "Grep", &needle, false);
    chains.observe(&second, &config, "Grep", &needle, false);
    chains.observe(&first, &config, "Grep", &needle, false);

    assert_eq!(chains.run_length(&first), 2);
    assert_eq!(chains.run_length(&second), 1);
}

#[test]
fn a_disabled_guard_observes_nothing() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-disabled", None);
    let config = RepeatToolConfig {
        enabled: false,
        ..RepeatToolConfig::default()
    };

    for _ in 0..10 {
        chains.observe(&key, &config, "Grep", &grep("needle"), false);
    }

    assert_eq!(chains.run_length(&key), 0);
    assert!(chains.take_reminders(&key).is_empty());
}

/// The preview bounds what a reminder quotes. It must not bound what the chain
/// compares, or two `Write` calls with the same first 500 characters and
/// different payloads would look like a loop.
#[test]
fn the_chain_compares_the_full_arguments_not_the_preview() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-full-compare", None);
    let config = RepeatToolConfig {
        arguments_preview_chars: 20,
        ..RepeatToolConfig::default()
    };
    let shared = "x".repeat(200);

    chains.observe(
        &key,
        &config,
        "Write",
        &serde_json::json!({"content": format!("{shared}first")}),
        false,
    );
    chains.observe(
        &key,
        &config,
        "Write",
        &serde_json::json!({"content": format!("{shared}second")}),
        false,
    );

    assert_eq!(
        chains.run_length(&key),
        1,
        "payloads differing past the preview cut are different calls"
    );
}

#[test]
fn a_reminder_quotes_no_more_than_the_configured_preview() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-preview", None);
    let config = RepeatToolConfig {
        arguments_preview_chars: 32,
        ..RepeatToolConfig::default()
    };
    let input = serde_json::json!({"content": "y".repeat(4096)});

    for _ in 0..3 {
        chains.observe(&key, &config, "Write", &input, false);
    }

    let reminders = chains.take_reminders(&key);
    assert!(reminders[0].contains("more characters, not shown"));
    assert!(
        reminders[0].len() < 1024,
        "a looping payload must not ride into the next request"
    );
}

#[test]
fn draining_leaves_nothing_behind() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-drain", None);
    let config = config();

    for _ in 0..3 {
        chains.observe(&key, &config, "Grep", &grep("needle"), false);
    }

    assert_eq!(chains.take_reminders(&key).len(), 1);
    assert!(chains.take_reminders(&key).is_empty());
}

/// A reminder earned by a run is still owed even if the model changed tool
/// before anyone drained it.
#[test]
fn an_undrained_reminder_survives_the_run_that_earned_it_ending() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-survives", None);
    let config = config();

    for _ in 0..3 {
        chains.observe(&key, &config, "Grep", &grep("needle"), false);
    }
    chains.observe(
        &key,
        &config,
        "Read",
        &serde_json::json!({"file_path": "a"}),
        false,
    );

    assert_eq!(chains.take_reminders(&key).len(), 1);
}

#[test]
fn an_undrained_queue_stays_bounded() {
    let chains = RepeatToolChains::default();
    let key = ChainKey::new("session-bounded", None);
    let config = config();

    for round in 0..50 {
        let input = grep(&format!("needle-{round}"));
        for _ in 0..8 {
            chains.observe(&key, &config, "Grep", &input, false);
        }
    }

    assert_eq!(chains.take_reminders(&key).len(), MAX_PENDING_REMINDERS);
}

#[test]
fn the_number_of_tracked_agents_is_bounded() {
    let chains = RepeatToolChains::default();
    let config = config();
    let needle = grep("needle");

    for index in 0..(MAX_TRACKED_AGENTS + 64) {
        let key = ChainKey::new(&format!("session-{index}"), None);
        chains.observe(&key, &config, "Grep", &needle, false);
    }

    let tracked = chains.chains.lock().expect("registry lock").len();
    assert!(
        tracked <= MAX_TRACKED_AGENTS,
        "tracked {tracked} agents, cap is {MAX_TRACKED_AGENTS}"
    );
    let newest = ChainKey::new(&format!("session-{}", MAX_TRACKED_AGENTS + 63), None);
    assert_eq!(
        chains.run_length(&newest),
        1,
        "eviction must drop the oldest chain, not the newest"
    );
}
