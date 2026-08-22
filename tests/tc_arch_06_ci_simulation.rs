//! TC-ARCH-06 (US-ARCH-04, e2e): the negative control for `arch-lint.sh`.
//!
//! `tc_arch_02` asserts the lint passes on the clean tree and that each rule
//! inspected something on the way. That is one half. This is the other: proof
//! that the lint *fails* when a violation is present. Without it, "exit 0 on a
//! clean tree, with non-zero counts" is still consistent with a rule whose
//! forbidden-pattern match can never fire.
//!
//! ## Why this was rewritten
//!
//! It was `#[ignore]`d and could not have passed if it were not. It located a
//! `BEGIN INPUT_HANDLER` marker in `src/main.rs` and injected a violation after
//! it — but those markers were deleted when the input handler moved out of
//! `main.rs`, so the `.expect("BEGIN INPUT_HANDLER marker not found")` was the
//! only outcome available to it. A permanently-ignored test that panics on its
//! first line the moment anyone un-ignores it is not coverage; it reads as
//! coverage, which is worse, because the one test that proves the lint has teeth
//! was the one nobody was running.
//!
//! It also mutated the real `src/main.rs` in the working tree and copied files
//! that no rule looks at any more (`crates/archon-core/src/agent.rs`,
//! `crates/archon-tui/src/app.rs`). Both are gone. The fixture below is a
//! synthetic tree built in a temp dir: nothing in the repository is touched, and
//! nothing depends on the current contents of the real sources, so the test
//! keeps working when the region moves again.
//!
//! `arch-lint.sh` resolves its own repo root from `$BASH_SOURCE/../..`, so
//! dropping a copy of the script at `<tmp>/scripts/lint/arch-lint.sh` makes the
//! temp tree its entire world.
//!
//! ## One injection per rule, and why
//!
//! The first rewrite used a single injected `handle_input` calling
//! `process_message`. Measured: replacing rule 1's forbidden-pattern grep with
//! `ZZZ_NEVER_MATCHES` left that test still reporting `ok`, because rule 3's
//! function-name convention covered the same line. A negative control that any
//! one of three rules can satisfy does not prove any of the three. Each rule now
//! gets a violation only it can catch, and each assertion names the rule it
//! expects in the failure output.

use std::path::Path;
use std::process::Command;

/// The smallest tree `arch-lint.sh` will accept as non-vacuous:
///
/// * both input-handler region directories exist and hold a `.rs` source
/// * the region names `spawn_turn` (rule 1's anchor: turn dispatch still lives
///   here) and contains at least one `.await` (rule 1's candidate sites)
/// * `crates/archon-core/src/agent/events.rs` makes the awaited bounded send
///   that rule 2 requires
///
/// Written as literals rather than copied from the repo so that a change to the
/// real sources cannot silently turn this fixture into something the lint skips.
fn write_clean_fixture(root: &Path) {
    let session_loop = root.join("src/session_loop");
    std::fs::create_dir_all(&session_loop).expect("create src/session_loop");
    std::fs::write(
        session_loop.join("mod.rs"),
        r#"
pub async fn dispatch_user_prompt(prompt: String) {
    // The architecture: hand agent work to the dispatcher, never drive it here.
    let _ = DISPATCHER.spawn_turn(prompt, runner());
    input_tx.send(()).await.ok();
}
"#,
    )
    .expect("write session_loop/mod.rs");

    let event_loop = root.join("crates/archon-tui/src/event_loop");
    std::fs::create_dir_all(&event_loop).expect("create event_loop");
    std::fs::write(
        event_loop.join("mod.rs"),
        r#"
pub async fn handle_key_event(key: Key) {
    let _ = dispatcher.spawn_turn(key.into(), runner());
    tx.send(key).await.ok();
}
"#,
    )
    .expect("write event_loop/mod.rs");

    let agent = root.join("crates/archon-core/src/agent");
    std::fs::create_dir_all(&agent).expect("create agent dir");
    std::fs::write(
        agent.join("events.rs"),
        r#"
pub(super) async fn send_event(&self, event: AgentEvent) {
    let timestamped = TimestampedEvent { sent_at: Instant::now(), inner: event };
    let _ = self.event_tx.send(timestamped).await;
}
"#,
    )
    .expect("write events.rs");

    let lint_dir = root.join("scripts/lint");
    std::fs::create_dir_all(&lint_dir).expect("create scripts/lint");
    let real_script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/lint/arch-lint.sh");
    assert!(
        real_script.exists(),
        "TC-ARCH-06: {real_script:?} not found — nothing to simulate"
    );
    std::fs::copy(&real_script, lint_dir.join("arch-lint.sh")).expect("copy arch-lint.sh");
}

fn run_lint(root: &Path) -> (bool, String) {
    let output = Command::new(bash_program())
        .arg(bash_path(&root.join("scripts/lint/arch-lint.sh")))
        .output()
        .expect("execute arch-lint.sh");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// Build the clean fixture, assert the lint passes on it and that every rule
/// reported, then hand the tree back for one injection.
///
/// The clean leg is the control. Without it a non-zero exit after an injection
/// proves nothing: a malformed fixture fails the lint just as loudly, and that is
/// exactly how a negative control stops testing the thing it names.
fn fixture_that_passes_clean(tmp: &tempfile::TempDir) {
    let root = tmp.path();
    write_clean_fixture(root);

    let (clean_ok, clean_output) = run_lint(root);
    assert!(
        clean_ok,
        "TC-ARCH-06: the synthetic clean tree must pass arch-lint before any \
         injection means anything.\n{clean_output}"
    );
    for rule in [1u32, 2, 3] {
        assert!(
            clean_output.contains(&format!("rule={rule} ")),
            "TC-ARCH-06: rule {rule} did not report on the clean fixture, so a \
             failure below cannot be attributed to it.\n{clean_output}"
        );
    }
}

fn assert_lint_rejects(root: &Path, expected_rule_name: &str, expected_evidence: &str) {
    let (ok, output) = run_lint(root);

    assert!(
        !ok,
        "TC-ARCH-06: arch-lint.sh exited 0 on an injected violation of \
         '{expected_rule_name}'. The rule reports counts but its forbidden-pattern \
         match never fires.\n{output}"
    );
    // Match the *failure* line, not the rule name anywhere in the output. Every
    // rule that passes echoes its own name on the way through
    // (`arch-lint: rule=1 ... name=<NAME>`), so a bare `contains(name)` is
    // satisfied by the rule succeeding. Measured: with rule 1's forbidden-pattern
    // grep replaced by `ZZZ_NEVER_MATCHES`, rule 3 caught the same line, rule 1
    // still printed its success echo, and this assertion passed on a dead rule.
    let failure_line = format!("FORBIDDEN pattern for rule '{expected_rule_name}'");
    assert!(
        output.contains(&failure_line),
        "TC-ARCH-06: the lint failed, but not for '{expected_rule_name}' — so this \
         rule's matcher is still unproven and something else happened to \
         fail.\n{output}"
    );
    assert!(
        output.contains(expected_evidence),
        "TC-ARCH-06: the failure must name the offending code \
         ('{expected_evidence}'), not just exit non-zero.\n{output}"
    );
    assert!(
        output.contains("spawn-everything-philosophy.md"),
        "TC-ARCH-06: the failure must point at the guideline it enforces.\n{output}"
    );
}

/// Rule 1: a synchronous agent turn anywhere in the input-handler region.
///
/// The violation goes in its own file, in a function deliberately *not* named
/// like an input handler, so neither rule 3's function-name convention nor its
/// 200-line window can also cover it. Otherwise a dead rule 1 hides behind rule 3
/// firing on the same line — which is exactly what happened with the earlier
/// single-injection version.
#[test]
fn rule_1_catches_process_message_in_the_region() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    fixture_that_passes_clean(&tmp);

    let target = tmp.path().join("src/session_loop/turn.rs");
    std::fs::write(
        &target,
        "pub async fn drive_the_turn() { agent.process_message(&prompt).await; }\n",
    )
    .expect("inject violation");

    assert_lint_rejects(
        tmp.path(),
        "no .await on agent work in input handler (D1)",
        "process_message",
    );
}

/// Rule 2: the agent event transport stops awaiting bounded capacity.
#[test]
fn rule_2_catches_an_unawaited_event_send() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    fixture_that_passes_clean(&tmp);

    let target = tmp.path().join("crates/archon-core/src/agent/events.rs");
    std::fs::write(
        &target,
        r#"
pub(super) async fn send_event(&self, event: AgentEvent) {
    let timestamped = TimestampedEvent { sent_at: Instant::now(), inner: event };
    // Unbounded, lossy: capacity is never awaited.
    let _ = self.event_tx.try_send(timestamped);
}
"#,
    )
    .expect("rewrite events.rs");

    assert_lint_rejects(
        tmp.path(),
        "Agent event transport must await bounded capacity (D3)",
        "events.rs",
    );
}

/// Rule 3: an input-handler function awaits a previous turn's handle.
///
/// This one carries no `process_message`, so rule 1 cannot cover for it.
#[test]
fn rule_3_catches_an_awaited_turn_handle_in_a_handler() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    fixture_that_passes_clean(&tmp);

    let target = tmp.path().join("crates/archon-tui/src/event_loop/mod.rs");
    std::fs::write(
        &target,
        r#"
pub async fn handle_key_event(key: Key) {
    let _ = dispatcher.spawn_turn(key.into(), runner());
    // Serialises the loop on the previous turn just as effectively as an
    // inline process_message would.
    let _ = turn_handle.await;
}
"#,
    )
    .expect("rewrite event_loop/mod.rs");

    assert_lint_rejects(
        tmp.path(),
        "no .await on agent work in input handler function (D1 broad)",
        "turn_handle",
    );
}

/// Locate a bash that actually exists.
///
/// Git for Windows ships `bash.exe` but its installer only adds `<git>\cmd` to
/// PATH -- the directory with `git.exe` and not `bash.exe` -- so a bare
/// `Command::new("bash")` fails with "program not found" on an otherwise
/// correctly set up machine. Same helper as `tc_arch_02` / `tc_arch_05`.
fn bash_program() -> std::ffi::OsString {
    #[cfg(windows)]
    {
        if let Ok(output) = Command::new("where").arg("git").output() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let git = Path::new(line.trim());
                if git.as_os_str().is_empty() {
                    continue;
                }
                if let Some(bash) = bash_near(git) {
                    return bash.into_os_string();
                }
            }
        }
        "bash".into()
    }
    #[cfg(not(windows))]
    {
        "bash".into()
    }
}

/// First `bash.exe` found in any ancestor of `git`, checking both layouts Git
/// for Windows ships (`<root>\bin` and `<root>\usr\bin`).
#[cfg(windows)]
fn bash_near(git: &Path) -> Option<std::path::PathBuf> {
    const RELATIVE: [&[&str]; 2] = [&["bin", "bash.exe"], &["usr", "bin", "bash.exe"]];
    for root in git.ancestors() {
        for parts in RELATIVE {
            let mut candidate = root.to_path_buf();
            for part in parts {
                candidate.push(part);
            }
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// A path bash will accept: Git's bash consumes backslashes as escapes.
fn bash_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
