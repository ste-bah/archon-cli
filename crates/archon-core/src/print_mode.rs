use crate::input_format::InputFormat;
use crate::output_format::OutputFormat;

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

/// Successful completion.
pub const EXIT_SUCCESS: i32 = 0;
/// General error (API failure, config error, etc.).
pub const EXIT_ERROR: i32 = 1;
/// Budget limit exceeded.
pub const EXIT_BUDGET_EXCEEDED: i32 = 2;
/// Maximum turn count exceeded.
pub const EXIT_MAX_TURNS: i32 = 3;
/// A tool the run needed was denied by permission policy.
///
/// Distinct from `EXIT_ERROR` because the run did not fail — it was not
/// permitted. The remedy is a permission mode or an allowlist entry, not a
/// retry, and a caller scripting `-p` should be able to tell those apart.
pub const EXIT_PERMISSION_DENIED: i32 = 4;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for a single print-mode invocation.
pub struct PrintModeConfig {
    /// The user query to process.
    pub query: String,
    /// How to format output (text, json, stream-json).
    pub output_format: OutputFormat,
    /// How to parse input when reading from stdin.
    pub input_format: InputFormat,
    /// Maximum number of agentic turns before forced exit.
    pub max_turns: Option<u32>,
    /// Maximum spend in USD before forced exit.
    pub max_budget_usd: Option<f64>,
    /// If true, do not persist the session to disk.
    pub no_session_persistence: bool,
    /// Optional JSON schema string to validate the final assistant output against.
    pub json_schema: Option<String>,
}

// ---------------------------------------------------------------------------
// Print mode runner
// ---------------------------------------------------------------------------

use std::io::Write as _;
use std::sync::Arc;

use crate::agent::{Agent, AgentEvent, TimestampedEvent};
use crate::config::ArchonConfig;
use crate::output_format::{format_agent_event, format_json_result_with_diagnostics};

/// Run print mode: process a single query, emit output, and return an exit code.
///
/// This function does not start a TUI. All assistant text goes to stdout;
/// tool output and diagnostics go to stderr.
pub async fn run_print_mode(
    config: PrintModeConfig,
    _archon_config: &ArchonConfig,
    agent: &mut Agent,
    event_rx: tokio::sync::mpsc::Receiver<TimestampedEvent>,
) -> i32 {
    run_print_mode_with_writers(
        config,
        _archon_config,
        agent,
        event_rx,
        SharedWriter::new(std::io::stdout()),
        SharedWriter::new(std::io::stderr()),
    )
    .await
}

#[derive(Clone)]
struct SharedWriter(Arc<std::sync::Mutex<Box<dyn std::io::Write + Send>>>);

impl SharedWriter {
    fn new(writer: impl std::io::Write + Send + 'static) -> Self {
        Self(Arc::new(std::sync::Mutex::new(Box::new(writer))))
    }
}

impl std::io::Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .flush()
    }
}

async fn run_print_mode_with_writers(
    config: PrintModeConfig,
    _archon_config: &ArchonConfig,
    agent: &mut Agent,
    mut event_rx: tokio::sync::mpsc::Receiver<TimestampedEvent>,
    stdout_writer: SharedWriter,
    stderr_writer: SharedWriter,
) -> i32 {
    let query = config.query.clone();
    let output_format = config.output_format.clone();
    let max_turns = config.max_turns;
    let max_budget = config.max_budget_usd;
    let json_schema = config.json_schema.clone();

    // Accumulate text for json mode final output
    let accumulated_text = Arc::new(tokio::sync::Mutex::new(String::new()));
    let accumulated_for_task = Arc::clone(&accumulated_text);

    // Track turns and cost
    let turn_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let turn_count_for_events = Arc::clone(&turn_count);
    let total_input_tokens = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total_output_tokens = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total_input_for_events = Arc::clone(&total_input_tokens);
    let total_output_for_events = Arc::clone(&total_output_tokens);

    // Track if budget/turn limit was hit
    let limit_exit_code = Arc::new(std::sync::atomic::AtomicI32::new(EXIT_SUCCESS));
    let limit_exit_for_events = Arc::clone(&limit_exit_code);

    let fmt_clone = output_format.clone();
    let stdout_for_events = stdout_writer.clone();
    let stderr_for_events = stderr_writer.clone();

    // Spawn event consumer that writes to stdout/stderr
    let event_handle = tokio::spawn(async move {
        let mut stdout = stdout_for_events;
        let mut stderr = stderr_for_events;

        while let Some(ts) = event_rx.recv().await {
            let event = ts.inner;
            // Track turn completions
            if let AgentEvent::TurnComplete {
                input_tokens,
                output_tokens,
                ..
            } = &event
            {
                turn_count_for_events.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                total_input_for_events
                    .fetch_add(*input_tokens, std::sync::atomic::Ordering::Relaxed);
                total_output_for_events
                    .fetch_add(*output_tokens, std::sync::atomic::Ordering::Relaxed);

                // Check turn limit
                if let Some(max) = max_turns {
                    let current = turn_count_for_events.load(std::sync::atomic::Ordering::Relaxed);
                    if current >= max {
                        limit_exit_for_events
                            .store(EXIT_MAX_TURNS, std::sync::atomic::Ordering::Relaxed);
                    }
                }

                // Check budget limit
                if let Some(budget) = max_budget {
                    let inp =
                        total_input_for_events.load(std::sync::atomic::Ordering::Relaxed) as f64;
                    let out =
                        total_output_for_events.load(std::sync::atomic::Ordering::Relaxed) as f64;
                    let cost = (inp * 3.0 + out * 15.0) / 1_000_000.0;
                    if cost >= budget {
                        limit_exit_for_events
                            .store(EXIT_BUDGET_EXCEEDED, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }

            // Accumulate text for json mode
            if let AgentEvent::TextDelta(ref text) = event {
                let mut acc = accumulated_for_task.lock().await;
                acc.push_str(text);
            }

            // Write tool output to stderr in all modes
            if let AgentEvent::ToolCallComplete {
                ref name,
                ref result,
                ..
            } = event
            {
                let _ = writeln!(
                    stderr,
                    "[tool:{name}] {}",
                    if result.is_error { "ERROR: " } else { "" }
                );
                let _ = writeln!(stderr, "{}", result.content);
            }

            // Write formatted output
            if matches!(&event, AgentEvent::AsyncHookDiagnostic(_)) {
                continue;
            }
            if let Some(output) = format_agent_event(&event, &fmt_clone) {
                let _ = stdout.write_all(output.as_bytes());
                let _ = stdout.flush();
            }
        }
    });

    // Process the query through the agent
    let process_result = agent.process_message(&query).await;

    // Close the event channel so the consumer task finishes
    agent.close_event_channel();
    let _ = event_handle.await;
    let async_diagnostics = agent.close_async_hook_diagnostics();
    if output_format == OutputFormat::Text {
        let mut stderr = stderr_writer.clone();
        write_async_diagnostics(&mut stderr, &async_diagnostics);
    }

    // A denied tool is not a failed turn: `process_result` is Ok, the agent
    // reports the denial in prose, and print mode used to exit 0 having done
    // none of what was asked. Observed with `-p "/workflow-prd-spec ..."` —
    // the skill was denied, nothing was written, and the exit code said
    // success, which any script wrapping this would have believed.
    //
    // Keyed on the agent's typed denial log rather than on the message text,
    // and checked BEFORE the error paths so a run that was blocked reports
    // being blocked rather than whatever happened afterwards.
    let denials = {
        let log = agent.denial_log.lock().await;
        log.recent(usize::MAX).to_vec()
    };
    if !denials.is_empty() {
        let mut stderr = stderr_writer.clone();
        let _ = writeln!(
            stderr,
            "Error: {} tool call(s) denied by permission policy; the request was not carried out.",
            denials.len()
        );
        for entry in denials.iter().take(5) {
            let _ = writeln!(stderr, "  - {}: {}", entry.tool_name, entry.reason);
        }
        return EXIT_PERMISSION_DENIED;
    }

    // Check for agent errors
    if let Err(e) = process_result {
        let mut stderr = stderr_writer.clone();
        let _ = writeln!(stderr, "Error: {e}");
        return EXIT_ERROR;
    }

    // Check if a limit was hit
    let limit_code = limit_exit_code.load(std::sync::atomic::Ordering::Relaxed);
    if limit_code != EXIT_SUCCESS {
        let mut stderr = stderr_writer.clone();
        match limit_code {
            EXIT_MAX_TURNS => {
                let _ = writeln!(stderr, "Maximum turn limit reached");
            }
            EXIT_BUDGET_EXCEEDED => {
                let _ = writeln!(stderr, "Budget limit exceeded");
            }
            _ => {}
        }
        // In json mode, still output what we have
        if output_format == OutputFormat::Json {
            let text = accumulated_text.lock().await;
            let usage = archon_llm::types::Usage {
                input_tokens: total_input_tokens.load(std::sync::atomic::Ordering::Relaxed),
                output_tokens: total_output_tokens.load(std::sync::atomic::Ordering::Relaxed),
                ..Default::default()
            };
            let inp = usage.input_tokens as f64;
            let out = usage.output_tokens as f64;
            let cost = (inp * 3.0 + out * 15.0) / 1_000_000.0;
            let json =
                format_json_result_with_diagnostics(&text, &usage, cost, Some(&async_diagnostics));
            let mut stdout = stdout_writer.clone();
            let _ = stdout.write_all(json.as_bytes());
            let _ = stdout.write_all(b"\n");
        }
        return limit_code;
    }

    // Json mode: emit final result
    if output_format == OutputFormat::Json {
        let text = accumulated_text.lock().await;
        let usage = archon_llm::types::Usage {
            input_tokens: total_input_tokens.load(std::sync::atomic::Ordering::Relaxed),
            output_tokens: total_output_tokens.load(std::sync::atomic::Ordering::Relaxed),
            ..Default::default()
        };
        let inp = usage.input_tokens as f64;
        let out = usage.output_tokens as f64;
        let cost = (inp * 3.0 + out * 15.0) / 1_000_000.0;
        let json =
            format_json_result_with_diagnostics(&text, &usage, cost, Some(&async_diagnostics));
        let mut stdout = stdout_writer.clone();
        let _ = stdout.write_all(json.as_bytes());
        let _ = stdout.write_all(b"\n");
    }

    // JSON schema validation (CLI-227)
    if let Some(ref schema) = json_schema {
        let text = accumulated_text.lock().await;
        let mut stdout = stdout_writer.clone();
        let mut stderr = stderr_writer.clone();

        match crate::schema_validation::extract_json(&text) {
            Some(extracted) => {
                match crate::schema_validation::validate_json_schema(&extracted, schema) {
                    Ok(()) => {
                        // Valid: output the extracted JSON
                        let _ = stdout.write_all(extracted.as_bytes());
                        let _ = stdout.write_all(b"\n");
                        let _ = stdout.flush();
                    }
                    Err(errors) => {
                        let _ = writeln!(stderr, "JSON schema validation failed:");
                        for err in &errors {
                            let _ = writeln!(stderr, "  - {err}");
                        }
                        return EXIT_ERROR;
                    }
                }
            }
            None => {
                let _ = writeln!(
                    stderr,
                    "JSON schema validation failed: no JSON found in assistant output"
                );
                return EXIT_ERROR;
            }
        }
    }

    EXIT_SUCCESS
}

fn write_async_diagnostics(
    stderr: &mut impl std::io::Write,
    batch: &crate::hooks::AsyncHookDiagnosticBatch,
) {
    for diagnostic in &batch.diagnostics {
        let _ = writeln!(
            stderr,
            "[async hook:{}:{} source={}] {}",
            diagnostic.event,
            diagnostic.outcome,
            diagnostic.source.as_deref().unwrap_or("unknown"),
            diagnostic.message
        );
    }
    if batch.dropped > 0 {
        let _ = writeln!(
            stderr,
            "[async hook diagnostics] {} older diagnostic(s) dropped",
            batch.dropped
        );
    }
}

#[cfg(test)]
#[path = "print_mode_async_hook_tests.rs"]
mod async_hook_diagnostic_tests;
