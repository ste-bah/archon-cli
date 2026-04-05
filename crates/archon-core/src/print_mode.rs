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

use crate::agent::{Agent, AgentEvent};
use crate::config::ArchonConfig;
use crate::output_format::{format_agent_event, format_json_result};

/// Run print mode: process a single query, emit output, and return an exit code.
///
/// This function does not start a TUI. All assistant text goes to stdout;
/// tool output and diagnostics go to stderr.
pub async fn run_print_mode(
    config: PrintModeConfig,
    _archon_config: &ArchonConfig,
    agent: &mut Agent,
    mut event_rx: tokio::sync::mpsc::Receiver<AgentEvent>,
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

    // Spawn event consumer that writes to stdout/stderr
    let event_handle = tokio::spawn(async move {
        let mut stdout = std::io::stdout();
        let mut stderr = std::io::stderr();

        while let Some(event) = event_rx.recv().await {
            // Track turn completions
            if let AgentEvent::TurnComplete {
                input_tokens,
                output_tokens,
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

    // Check for agent errors
    if let Err(e) = process_result {
        let mut stderr = std::io::stderr();
        let _ = writeln!(stderr, "Error: {e}");
        return EXIT_ERROR;
    }

    // Check if a limit was hit
    let limit_code = limit_exit_code.load(std::sync::atomic::Ordering::Relaxed);
    if limit_code != EXIT_SUCCESS {
        let mut stderr = std::io::stderr();
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
            let json = format_json_result(&text, &usage, cost);
            let _ = std::io::stdout().write_all(json.as_bytes());
            let _ = std::io::stdout().write_all(b"\n");
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
        let json = format_json_result(&text, &usage, cost);
        let _ = std::io::stdout().write_all(json.as_bytes());
        let _ = std::io::stdout().write_all(b"\n");
    }

    // JSON schema validation (CLI-227)
    if let Some(ref schema) = json_schema {
        let text = accumulated_text.lock().await;
        let mut stdout = std::io::stdout();
        let mut stderr = std::io::stderr();

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
