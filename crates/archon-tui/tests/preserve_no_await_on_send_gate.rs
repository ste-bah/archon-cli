//! TASK-TUI-900 — PRESERVE gate: ban `.await` on agent-event producer `send()`.
//!
//! ## Purpose (ERR-TUI-001 regression guard)
//!
//! ERR-TUI-001 was the pre-fix deadlock pattern where the TUI awaited
//! `agent_event_tx.send(...).await` on a bounded `mpsc::Sender`. Under
//! load the channel filled, the producer blocked, and the whole event
//! loop froze. SPEC-TUI-EVENTCHANNEL fixed it by switching the
//! producer to `tokio::sync::mpsc::UnboundedSender`, whose `.send(...)`
//! is non-async (returns `Result<(), SendError>` immediately). That
//! positive refactor is owned by earlier tasks; this test owns the
//! **standing regression guard** so nobody re-introduces the pattern
//! by flipping the sender back to bounded and `.await`-ing it.
//!
//! ## How it works
//!
//! Pure-Rust static scanner (no `rg`/`grep` subprocess — those aren't
//! reliably on PATH in the Claude-hosted CI; see the pattern used in
//! `lint_no_inline_agent_await.rs` for the same rationale). Walks
//! `crates/archon-tui/src/` recursively, reads every `.rs` file whose
//! **file-name** matches one of the five patterns listed in
//! TASK-TUI-900 scope (`*event*.rs`, `*channel*.rs`, `*agent*.rs`,
//! `*input*.rs`, `*subagent*.rs`), and looks for the semantic
//! pattern:
//!
//! ```text
//! <ident ending in _tx | _sender | _channel>.send(<balanced parens>) <ws> .await
//! ```
//!
//! The scanner uses a **balanced-paren walk** (not a regex) to match
//! the `.send(...)` call, which means nested calls like
//! `tx.send(format!("{}", x)).await` are caught correctly. A pure
//! `[^)]*` regex would produce a false-negative on those callsites,
//! which is exactly the shape some of the real event sends take.
//!
//! Whitespace between the closing `)` and `.await` is skipped
//! including newlines, so continuations such as:
//!
//! ```ignore
//! agent_event_tx.send(evt)
//!     .await;
//! ```
//!
//! fire just the same as single-line hits. Every offender is
//! collected; the test panics at the end with a full list (not
//! first-match-wins) so the author sees every callsite they need to
//! fix.
//!
//! ## Whitelist (documented deviation)
//!
//! The regex `(_tx|_sender|_channel)` is deliberately generic; it
//! catches every sender-suffixed identifier. That includes
//! `input_tx` in `event_loop.rs`, which is a *bounded* `mpsc::Sender`
//! by design — it is the **user-input** queue (TUI → input loop),
//! not the agent-event producer path. Awaiting it applies
//! backpressure to the user's keyboard input, which is the intended
//! contract. ERR-TUI-001 is about events flowing **from** the agent
//! **to** the TUI, not from keystrokes into the dispatcher.
//!
//! Rather than narrow the regex (which would weaken the guard and
//! risk missing a real regression), we keep the broad pattern and
//! whitelist the specific identifier `input_tx` with a justification
//! comment here. If anyone adds a new whitelisted identifier they
//! must (a) justify it in the spec, (b) document it below, and (c)
//! accept the reviewer burden — the whitelist is deliberately
//! small.
//!
//! ## Scope boundary
//!
//! Scans `crates/archon-tui/src/` only. `archon-core` and
//! `archon-tools` are out of scope per TASK-TUI-900 §Out of Scope.
//! The scope is enforced by reading `CARGO_MANIFEST_DIR` (which for
//! a test binary compiled from `crates/archon-tui/tests/*.rs` is
//! `<worktree>/crates/archon-tui`) and joining `src/`.

use std::fs;
use std::path::{Path, PathBuf};

/// File-name patterns (case-insensitive substring match on the file
/// stem) that define the scope of the gate per TASK-TUI-900 §In Scope.
///
/// The spec calls out five patterns — if you add a sixth, update the
/// spec (01-functional-spec.md SPEC-TUI-PRESERVE) first.
const SCOPED_PATTERNS: &[&str] = &[
    "event",    // matches event_loop.rs, events.rs, event_latency*.rs
    "channel",  // matches any future *channel*.rs under src/
    "agent",    // matches any *agent*.rs (agent_handle etc.)
    "input",    // matches input.rs
    "subagent", // matches future subagent*.rs
];

/// Identifiers whose `.send(...).await` is *intentional* and NOT part
/// of the ERR-TUI-001 agent-event producer path. See the module-level
/// "Whitelist" section for why each is exempt.
const WHITELISTED_SENDERS: &[&str] = &[
    // `input_tx` is the bounded TUI -> input-loop queue (user
    // keystrokes / slash commands). Awaiting it provides flow control
    // on user input, which is the designed behaviour — it is not the
    // agent-event channel that ERR-TUI-001 fixed.
    "input_tx",
];

/// Returns `true` if the file-name (basename without extension)
/// contains one of the `SCOPED_PATTERNS` substrings. Case-insensitive.
fn file_in_scope(path: &Path) -> bool {
    let name = match path.file_name().and_then(|s| s.to_str()) {
        Some(n) => n.to_ascii_lowercase(),
        None => return false,
    };
    // Only .rs files — we do not scan .md / .toml etc.
    if !name.ends_with(".rs") {
        return false;
    }
    SCOPED_PATTERNS.iter().any(|p| name.contains(p))
}

/// Recursively walks `root`, returning every `.rs` file whose basename
/// matches one of the scoped patterns. Does NOT follow symlinks — the
/// TUI tree has no symlinks and following them would be a footgun.
fn collect_scoped_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            // Unreadable directory is a test failure signal, but we
            // collect what we can and let the outer test assert
            // non-empty file list (acts as a sanity check).
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let ty = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ty.is_symlink() {
                continue;
            }
            if ty.is_dir() {
                stack.push(p);
            } else if ty.is_file() && file_in_scope(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Per-line / per-window hit record for the failure message.
#[derive(Debug)]
struct Offender {
    file: PathBuf,
    // 1-indexed line number of the LINE THAT CONTAINS `.send(`.
    line: usize,
    // The offending text (trimmed). For multi-line hits we join the
    // two-line window with a literal "\n" in the message.
    snippet: String,
}

/// Scans one file and returns every offender (may be empty).
///
/// Implementation:
/// - Find each occurrence of `.send(` in the source.
/// - Walk backward from the `.` to extract the preceding identifier
///   (alphanumeric+underscore). If it doesn't end in one of the
///   `_tx` / `_sender` / `_channel` suffixes, skip.
/// - If the identifier is whitelisted, skip.
/// - Walk forward from the opening `(` counting paren depth until
///   the matching `)`. This correctly handles nested calls like
///   `tx.send(format!("{}", x)).await`, which a pure regex with
///   `[^)]*` would miss (false-negative).
/// - After the matching `)`, skip whitespace/newlines and check for
///   `.await`. The whitespace skip spans newlines, which gives us the
///   multi-line continuation behaviour the spec requires without
///   needing an explicit sliding window.
/// - Report the 1-indexed line number of the line containing `.send(`.
///
/// We intentionally do NOT try to strip comments or string literals —
/// a `.await` inside a string literal that also contains `_tx.send(`
/// is sufficiently contrived that we'd rather fire a false positive
/// than miss a real regression. If this ever bites a legitimate case,
/// add the specific identifier to `WHITELISTED_SENDERS`.
fn scan_file(path: &Path) -> Vec<Offender> {
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    // Precompute line-start byte offsets so we can convert byte
    // position -> 1-indexed line number in O(log n).
    let mut line_starts: Vec<usize> = vec![0];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let byte_to_line = |byte: usize| -> usize {
        // Binary search: largest line_start <= byte.
        match line_starts.binary_search(&byte) {
            Ok(idx) => idx + 1,
            Err(idx) => idx, // idx is insertion point; line is idx-1+1 = idx
        }
    };

    let bytes = src.as_bytes();
    let mut offenders: Vec<Offender> = Vec::new();
    let needle = b".send(";
    let mut cursor = 0;
    'outer: while cursor + needle.len() <= bytes.len() {
        // Find next `.send(` occurrence.
        let rel = match bytes[cursor..]
            .windows(needle.len())
            .position(|w| w == needle)
        {
            Some(r) => r,
            None => break,
        };
        let dot_pos = cursor + rel;
        let open_paren = dot_pos + 5; // position of `(` in `.send(`
        cursor = dot_pos + 1; // advance at least one byte so we don't loop

        // 1. Extract the identifier immediately before `.send(`.
        //    Walk backwards from `dot_pos - 1` while the byte is an
        //    ASCII identifier char.
        let ident_end = dot_pos;
        let mut ident_start = ident_end;
        while ident_start > 0 {
            let b = bytes[ident_start - 1];
            let is_ident = b.is_ascii_alphanumeric() || b == b'_';
            if !is_ident {
                break;
            }
            ident_start -= 1;
        }
        if ident_start == ident_end {
            // No identifier — e.g. `).send(` (method chain). Skip:
            // this guard only covers direct `<ident>.send()` callsites.
            // A method-chained `foo().send().await` is a different
            // shape and would be caught only if explicitly added.
            continue;
        }
        let ident = &src[ident_start..ident_end];

        // First char must be a non-digit (valid Rust ident).
        let first = bytes[ident_start];
        if first.is_ascii_digit() {
            continue;
        }

        // 2. Must end with one of the tracked suffixes.
        let matches_suffix =
            ident.ends_with("_tx") || ident.ends_with("_sender") || ident.ends_with("_channel");
        if !matches_suffix {
            continue;
        }

        // 3. Whitelist check.
        if WHITELISTED_SENDERS.contains(&ident) {
            continue;
        }

        // 4. Find the matching `)` for the `.send(` opening paren by
        //    counting depth. Bail out if unmatched (malformed source).
        let mut depth: i32 = 1;
        let mut i = open_paren + 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            // Unmatched paren — skip this occurrence.
            continue 'outer;
        }
        // `i` now points one byte past the matching `)`.
        let close_paren = i - 1;

        // 5. Skip whitespace (incl. newlines) after the closing paren.
        let mut j = close_paren + 1;
        while j < bytes.len() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        // 6. Check for `.await` token.
        let tail = b".await";
        if j + tail.len() > bytes.len() {
            continue;
        }
        if &bytes[j..j + tail.len()] != tail {
            continue;
        }
        // Require a non-ident boundary after `.await` so we don't
        // accidentally match `.awaited` or similar.
        let after = j + tail.len();
        if after < bytes.len() {
            let b = bytes[after];
            if b.is_ascii_alphanumeric() || b == b'_' {
                continue;
            }
        }

        // 7. Record the hit.
        let line = byte_to_line(dot_pos);
        // Snippet: from the start of the line to `.await`, trimmed and
        // collapsed so the failure message stays compact even for
        // multi-line continuations.
        let line_start = line_starts[line - 1];
        let snippet_raw = &src[line_start..after.min(src.len())];
        let snippet: String = snippet_raw
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        offenders.push(Offender {
            file: path.to_path_buf(),
            line,
            snippet,
        });
    }

    // De-duplicate by line (a defensive measure — the forward scan
    // shouldn't produce duplicates, but cheap to be safe).
    offenders.sort_by(|a, b| a.line.cmp(&b.line));
    offenders.dedup_by(|a, b| a.line == b.line);
    offenders
}

fn tui_src_root() -> PathBuf {
    // CARGO_MANIFEST_DIR for a test compiled from
    // `crates/archon-tui/tests/*.rs` resolves to the archon-tui
    // crate root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

#[test]
fn preserve_no_await_on_send_in_agent_event_producer_path() {
    let root = tui_src_root();
    assert!(
        root.is_dir(),
        "TUI-900 gate: expected `{}` to exist; the scope is hard-coded \
         to the archon-tui crate and something has moved the source tree.",
        root.display()
    );

    let files = collect_scoped_files(&root);
    assert!(
        !files.is_empty(),
        "TUI-900 gate: scanned `{}` and found no files matching any of \
         the scoped patterns {:?}. Either the crate is empty (which \
         is impossible — we depend on it) or the patterns have drifted \
         from the source tree; update SCOPED_PATTERNS and the spec.",
        root.display(),
        SCOPED_PATTERNS
    );

    let mut all_offenders: Vec<Offender> = Vec::new();
    for f in &files {
        all_offenders.extend(scan_file(f));
    }

    if all_offenders.is_empty() {
        return;
    }

    // Build a single, grep-friendly failure message. The `ERR-TUI-001`
    // marker is mandatory so reviewers and CI log-scrapers can key on
    // it. We also include the "VIOLATION" token so the negative test
    // in TASK-TUI-900 §Test Commands (`grep -c "VIOLATION"`) matches.
    let mut msg = String::new();
    msg.push_str(
        "TUI-900 VIOLATION (ERR-TUI-001 regression): `.await` found on a \
         producer `send()` call under crates/archon-tui/src/.\n\n\
         ERR-TUI-001 is the bounded-channel deadlock pattern. The \
         agent-event channel was migrated to UnboundedSender by \
         SPEC-TUI-EVENTCHANNEL specifically so `.send(...)` is \
         non-async. Re-introducing `.await` on a producer send \
         indicates someone has flipped the channel back to bounded or \
         switched to a different bounded producer — either way it's \
         the same deadlock class.\n\n\
         Fix: make the sender an UnboundedSender (or equivalent \
         non-blocking producer) and drop the `.await`. If you \
         genuinely need bounded backpressure on a NEW channel, add \
         the identifier to WHITELISTED_SENDERS in this test file with \
         a written justification, and update the spec at \
         01-functional-spec.md SPEC-TUI-PRESERVE.\n\n\
         Offending callsites:\n",
    );
    for o in &all_offenders {
        // Print the worktree-relative path when possible so the
        // message is portable across machines.
        let display_path = o
            .file
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .map(|p| PathBuf::from("crates/archon-tui").join(p))
            .unwrap_or_else(|_| o.file.clone());
        msg.push_str(&format!(
            "  {}:{}: {}\n",
            display_path.display(),
            o.line,
            o.snippet
        ));
    }
    panic!("{}", msg);
}

/// Sanity assertion: the scope really covers the patterns the spec
/// says it does. If someone edits SCOPED_PATTERNS and breaks a name
/// this fires immediately rather than silently narrowing the guard.
#[test]
fn preserve_scope_patterns_are_the_five_ticketed_ones() {
    // SPEC-TUI-PRESERVE AC-PRESERVE-01 requires exactly these five
    // patterns (validation criterion #3). Lock them down.
    assert_eq!(
        SCOPED_PATTERNS,
        &["event", "channel", "agent", "input", "subagent"],
        "TUI-900 gate: SCOPED_PATTERNS drifted from the spec. If you \
         are deliberately changing scope, update \
         01-functional-spec.md SPEC-TUI-PRESERVE first, then update \
         this assertion."
    );
}
