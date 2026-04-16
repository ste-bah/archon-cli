//! Slash command handler. Extracted from main.rs.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use anyhow::anyhow;
use archon_consciousness::rules::RulesEngine;
use archon_llm::effort::{self, EffortLevel, EffortState};
use archon_llm::fast_mode::FastModeState;
use archon_tools::task_manager;
use archon_tui::app::TuiEvent;
use crate::command::config::handle_config_command;
use crate::command::doctor::handle_doctor_command;
use crate::command::memory::handle_memory_command;
use crate::command::registry::CommandContext;
use crate::slash_context::SlashCommandContext;

/// Handle slash commands. Returns `true` if the command was recognized and handled.
pub(crate) async fn handle_slash_command(
    input: &str,
    fast_mode: &mut FastModeState,
    effort_state: &mut EffortState,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &mut SlashCommandContext,
) -> bool {
    // TASK-AGS-623 dispatcher gate (PATH A hybrid).
    //
    // Every slash input now flows through exactly one `Dispatcher::dispatch
    // call: parser → registry lookup → handler (currently no-op stubs from
    // TASK-AGS-622) or `TuiEvent::Error("Unknown command: /{name}")` on miss.
    // Recognized commands fall through to the legacy 43-arm match below,
    // which continues to perform the actual command bodies until TASK-AGS-624
    // migrates those bodies into the registry's stub `execute` methods.
    // Non-slash / empty / bare-`/` inputs short-circuit with `false` — the
    // same behaviour the TASK-AGS-621 parser gate provided.
    let mut __cmd_ctx = crate::command::registry::CommandContext {
        tui_tx: tui_tx.clone(),
    };
    let _ = ctx.dispatcher.dispatch(&mut __cmd_ctx, input);
    if !ctx.dispatcher.recognizes(input) {
        return false;
    }

    match input {
        "/fast" => {
            let new_state = fast_mode.toggle();
            ctx.fast_mode_shared.store(new_state, Ordering::Relaxed);
            let msg = if new_state {
                "Fast mode ENABLED. Responses will be faster but lower quality."
            } else {
                "Fast mode DISABLED. Back to normal quality."
            };
            let _ = tui_tx.send(TuiEvent::TextDelta(format!("\n{msg}\n"))).await;
            true
        }
        // /compact and /clear are handled inline in the input processor (need agent access)
        "/compact" | "/clear" => true,
        s if s == "/export" || s.starts_with("/export ") => true,
        "/thinking on" | "/thinking" => {
            ctx.show_thinking.store(true, Ordering::Relaxed);
            let _ = tui_tx.send(TuiEvent::ThinkingToggle(true)).await;
            let _ = tui_tx
                .send(TuiEvent::TextDelta("\nThinking display enabled.\n".into()))
                .await;
            true
        }
        "/thinking off" => {
            ctx.show_thinking.store(false, Ordering::Relaxed);
            let _ = tui_tx.send(TuiEvent::ThinkingToggle(false)).await;
            let _ = tui_tx
                .send(TuiEvent::TextDelta("\nThinking display disabled.\n".into()))
                .await;
            true
        }
        // ── /effort ────────────────────────────────────────────
        s if s.starts_with("/effort") => {
            let level_str = s.strip_prefix("/effort").unwrap_or("").trim();
            if level_str.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nCurrent effort level: {}\nUsage: /effort <high|medium|low>\n",
                        effort_state.level()
                    )))
                    .await;
            } else {
                match archon_tools::validation::validate_effort_level(level_str) {
                    Ok(validated) => {
                        // Safe: validated is always one of "high", "medium", "low"
                        let level = effort::parse_level(&validated)
                            .expect("validated effort level must parse");
                        effort_state.set_level(level);
                        *ctx.effort_level_shared.lock().await = level;
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nEffort level set to {level}.\n"
                            )))
                            .await;
                    }
                    Err(msg) => {
                        let _ = tui_tx.send(TuiEvent::Error(msg)).await;
                    }
                }
            }
            true
        }
        // ── /garden ────────────────────────────────────────────
        s if s == "/garden" || s.starts_with("/garden ") => {
            let sub = s.strip_prefix("/garden").unwrap_or("").trim();
            if sub == "stats" {
                match archon_memory::garden::format_garden_stats(ctx.memory.as_ref(), 10) {
                    Ok(stats) => {
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!("\n{stats}\n")))
                            .await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Garden stats failed: {e}")))
                            .await;
                    }
                }
            } else {
                match archon_memory::garden::consolidate(ctx.memory.as_ref(), &ctx.garden_config) {
                    Ok(report) => {
                        let formatted = report.format();
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!("\n{formatted}\n")))
                            .await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Garden consolidation failed: {e}")))
                            .await;
                    }
                }
            }
            true
        }
        // ── /model ─────────────────────────────────────────────
        s if s.starts_with("/model") => {
            let model_str = s.strip_prefix("/model").unwrap_or("").trim();
            if model_str.is_empty() {
                let current = {
                    let ov = ctx.model_override_shared.lock().await;
                    if ov.is_empty() {
                        ctx.default_model.clone()
                    } else {
                        ov.clone()
                    }
                };
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nCurrent model: {current}\nUsage: /model <name>\nShortcuts: opus, sonnet, haiku\n"
                    )))
                    .await;
            } else {
                match archon_tools::validation::validate_model_name(model_str) {
                    Ok(resolved) => {
                        *ctx.model_override_shared.lock().await = resolved.clone();
                        let _ = tui_tx.send(TuiEvent::ModelChanged(resolved.clone())).await;
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nModel switched to {resolved}.\n"
                            )))
                            .await;
                    }
                    Err(msg) => {
                        let _ = tui_tx.send(TuiEvent::Error(msg)).await;
                    }
                }
            }
            true
        }
        // ── /copy ───────────────────────────────────────────────
        "/copy" => {
            // Find the last assistant message content
            let last_response = ctx.last_assistant_response.lock().await;
            if last_response.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(
                        "\nNo assistant response to copy.\n".into(),
                    ))
                    .await;
            } else {
                // Detect clipboard tool by trying each directly
                let tool = if std::process::Command::new("which")
                    .arg("xclip")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    "xclip"
                } else if std::process::Command::new("which")
                    .arg("clip.exe")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    "clip.exe"
                } else if std::process::Command::new("which")
                    .arg("pbcopy")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    "pbcopy"
                } else {
                    "none"
                };

                let copied = match tool {
                    "xclip" => {
                        let mut child = std::process::Command::new("xclip")
                            .arg("-selection")
                            .arg("clipboard")
                            .stdin(std::process::Stdio::piped())
                            .spawn();
                        if let Ok(ref mut c) = child {
                            use std::io::Write;
                            if let Some(ref mut stdin) = c.stdin {
                                let _ = stdin.write_all(last_response.as_bytes());
                            }
                            let _ = c.wait();
                            true
                        } else {
                            false
                        }
                    }
                    "clip.exe" => {
                        let mut child = std::process::Command::new("clip.exe")
                            .stdin(std::process::Stdio::piped())
                            .spawn();
                        if let Ok(ref mut c) = child {
                            use std::io::Write;
                            if let Some(ref mut stdin) = c.stdin {
                                let _ = stdin.write_all(last_response.as_bytes());
                            }
                            let _ = c.wait();
                            true
                        } else {
                            false
                        }
                    }
                    "pbcopy" => {
                        let mut child = std::process::Command::new("pbcopy")
                            .stdin(std::process::Stdio::piped())
                            .spawn();
                        if let Ok(ref mut c) = child {
                            use std::io::Write;
                            if let Some(ref mut stdin) = c.stdin {
                                let _ = stdin.write_all(last_response.as_bytes());
                            }
                            let _ = c.wait();
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };

                if copied {
                    let chars = last_response.len();
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta(format!(
                            "\nCopied {chars} characters to clipboard.\n"
                        )))
                        .await;
                } else {
                    let _ = tui_tx.send(TuiEvent::Error(
                        "No clipboard tool found. Install xclip (Linux), or use clip.exe (WSL) / pbcopy (macOS).".into()
                    )).await;
                }
            }
            true
        }
        // ── /context ────────────────────────────────────────────
        "/context" => {
            let stats = ctx.session_stats.lock().await;
            let input_k = stats.input_tokens as f64 / 1000.0;
            let output_k = stats.output_tokens as f64 / 1000.0;

            // Estimate token counts from character sizes (~4 chars per token)
            let sys_prompt_tokens = ctx.system_prompt_chars as f64 / 4.0;
            let tool_def_tokens = ctx.tool_defs_chars as f64 / 4.0;

            // Conversation tokens: input tokens minus the fixed overhead
            // (system prompt + tools are sent every turn, so the last
            // input_tokens from the API already includes them).
            let fixed_overhead = sys_prompt_tokens + tool_def_tokens;
            let conversation_tokens = if stats.input_tokens > 0 {
                (stats.input_tokens as f64).max(fixed_overhead) - fixed_overhead
            } else {
                0.0
            };

            // Total estimated context = fixed overhead + conversation
            let total_context = fixed_overhead + conversation_tokens;

            let context_limit = 200_000.0_f64;
            let pct = (total_context / context_limit * 100.0).min(100.0);
            let bar_width = 40usize;
            let filled = (pct / 100.0 * bar_width as f64) as usize;
            let bar: String = format!(
                "[{}{}] {pct:.1}%",
                "#".repeat(filled),
                "-".repeat(bar_width.saturating_sub(filled))
            );

            // Format a token count nicely (e.g. 3.2k or 312)
            let fmt_tok = |t: f64| -> String {
                if t >= 1000.0 {
                    format!("{:.1}k", t / 1000.0)
                } else {
                    format!("{:.0}", t)
                }
            };

            let msg = format!(
                "\nContext window usage:\n\
                 {bar}\n\
                 \n\
                 System prompt:    ~{sys} tokens\n\
                 Tool definitions: ~{tools} tokens\n\
                 Conversation:     ~{conv} tokens\n\
                 Total context:    ~{total} / {limit}k tokens\n\
                 \n\
                 API usage this session:\n\
                 Input:  {input_k:.1}k tokens\n\
                 Output: {output_k:.1}k tokens\n\
                 Turns:  {turns}\n",
                sys = fmt_tok(sys_prompt_tokens),
                tools = fmt_tok(tool_def_tokens),
                conv = fmt_tok(conversation_tokens),
                total = fmt_tok(total_context),
                limit = context_limit as u64 / 1000,
                turns = stats.turn_count,
            );
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /status ────────────────────────────────────────────
        "/status" => {
            let stats = ctx.session_stats.lock().await;
            let current_model = {
                let ov = ctx.model_override_shared.lock().await;
                if ov.is_empty() {
                    ctx.default_model.clone()
                } else {
                    ov.clone()
                }
            };
            let perm_mode = ctx.permission_mode.lock().await;
            let fast = ctx.fast_mode_shared.load(Ordering::Relaxed);
            let effort = ctx.effort_level_shared.lock().await;
            let thinking_visible = ctx.show_thinking.load(Ordering::Relaxed);
            let thinking_str = if thinking_visible {
                "visible"
            } else {
                "hidden"
            };
            let in_k = stats.input_tokens as f64 / 1000.0;
            let out_k = stats.output_tokens as f64 / 1000.0;
            let msg = format!(
                "\n\
                 Model: {current_model}\n\
                 Mode: {perm_mode} (permissions)\n\
                 Fast mode: {fast_label}\n\
                 Effort: {effort}\n\
                 Thinking: {thinking_str}\n\
                 Session: {sid}\n\
                 Tokens: {in_k:.1}k in / {out_k:.1}k out\n\
                 Turns: {turns}\n",
                fast_label = if fast { "on" } else { "off" },
                effort = *effort,
                sid = &ctx.session_id[..8.min(ctx.session_id.len())],
                turns = stats.turn_count,
            );
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /cost ──────────────────────────────────────────────
        "/cost" => {
            let stats = ctx.session_stats.lock().await;
            let input_cost = stats.input_tokens as f64 * 3.0 / 1_000_000.0;
            let output_cost = stats.output_tokens as f64 * 15.0 / 1_000_000.0;
            let total = input_cost + output_cost;
            let warn = ctx.cost_config.warn_threshold;
            let hard = ctx.cost_config.hard_limit;
            let hard_label = if hard <= 0.0 {
                "$0.00 (disabled)".to_string()
            } else {
                format!("${hard:.2}")
            };
            let cache_line = stats.cache_stats.format_for_cost();
            let msg = format!(
                "\n\
                 Session cost: ${total:.2}\n\
                 Input tokens: {input_tok} (${input_cost:.2})\n\
                 Output tokens: {output_tok} (${output_cost:.2})\n\
                 {cache_line}\n\
                 Warn threshold: ${warn:.2}\n\
                 Hard limit: {hard_label}\n",
                input_tok = stats.input_tokens,
                output_tok = stats.output_tokens,
            );
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /permissions ───────────────────────────────────────
        s if s.starts_with("/permissions") => {
            let arg = s.strip_prefix("/permissions").unwrap_or("").trim();
            if arg.is_empty() {
                let mode = ctx.permission_mode.lock().await;
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nCurrent permission mode: {mode}\n\
                         Usage: /permissions <mode>\n\
                         Modes: default, acceptEdits, plan, auto, dontAsk, bypassPermissions\n\
                         Legacy aliases: ask -> default, yolo -> bypassPermissions\n"
                    )))
                    .await;
            } else {
                match archon_tools::validation::validate_permission_mode(arg) {
                    Ok(resolved)
                        if resolved == "bypassPermissions" && !ctx.allow_bypass_permissions =>
                    {
                        let _ = tui_tx
                            .send(TuiEvent::Error(
                                "bypassPermissions requires --allow-dangerously-skip-permissions flag".into(),
                            ))
                            .await;
                    }
                    Ok(resolved) => {
                        *ctx.permission_mode.lock().await = resolved.clone();
                        let _ = tui_tx
                            .send(TuiEvent::PermissionModeChanged(resolved.clone()))
                            .await;
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nPermission mode set to {resolved}.\n"
                            )))
                            .await;
                    }
                    Err(msg) => {
                        let _ = tui_tx.send(TuiEvent::Error(msg)).await;
                    }
                }
            }
            true
        }
        // ── /config [key] [value] ──────────────────────────────
        s if s == "/config" || s.starts_with("/config ") => {
            handle_config_command(s, tui_tx, ctx).await;
            true
        }
        // ── /memory [subcommand] ───────────────────────────────
        s if s == "/memory" || s.starts_with("/memory ") => {
            handle_memory_command(s, tui_tx, &ctx.memory).await;
            true
        }
        // ── /doctor ────────────────────────────────────────────
        "/doctor" => {
            handle_doctor_command(tui_tx, ctx).await;
            true
        }
        // ── /bug ───────────────────────────────────────────────
        "/bug" => {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(
                    "\nReport bugs at https://github.com/anthropics/archon/issues\n".into(),
                ))
                .await;
            true
        }
        // ── /diff ──────────────────────────────────────────────
        "/diff" => {
            handle_diff_command(tui_tx, &ctx.working_dir).await;
            true
        }
        // ── /denials ──────────────────────────────────────────
        "/denials" => {
            let log = ctx.denial_log.lock().await;
            let text = log.format_display(20);
            let _ = tui_tx
                .send(TuiEvent::TextDelta(format!("\n{text}\n")))
                .await;
            true
        }
        // ── /login ─────────────────────────────────────────────
        "/login" => {
            let cred_path = dirs::home_dir()
                .unwrap_or_default()
                .join(".archon")
                .join(".credentials.json");
            let mut msg = String::from("\nAuthentication status:\n");
            msg.push_str(&format!("  Method: {}\n", ctx.auth_label));
            if cred_path.exists() {
                msg.push_str(&format!("  Credentials: {}\n", cred_path.display()));
                msg.push_str("  Status: authenticated\n\n");
                msg.push_str("  To re-authenticate, run in another terminal:\n");
                msg.push_str("    archon login\n");
            } else {
                msg.push_str("  Credentials: not found\n");
                msg.push_str("  Status: using API key or not authenticated\n\n");
                msg.push_str("  To authenticate with OAuth:\n");
                msg.push_str("    1. Exit this session (Ctrl+D)\n");
                msg.push_str("    2. Run: archon login\n");
                msg.push_str("    3. Follow the browser flow\n");
                msg.push_str("    4. Restart archon\n");
            }
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /vim ───────────────────────────────────────────────
        "/vim" => {
            let _ = tui_tx.send(TuiEvent::VimToggle).await;
            let _ = tui_tx
                .send(TuiEvent::TextDelta(
                    "\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n".into(),
                ))
                .await;
            true
        }
        // ── /usage ────────────────────────────────────────────
        "/usage" => {
            // Same as /cost but with more detail — redirect
            let stats = ctx.session_stats.lock().await;
            let input_cost = stats.input_tokens as f64 * 3.0 / 1_000_000.0;
            let output_cost = stats.output_tokens as f64 * 15.0 / 1_000_000.0;
            let total = input_cost + output_cost;
            let cache_line = stats.cache_stats.format_for_cost();
            let msg = format!(
                "\nUsage summary:\n\
                 Turns:         {turns}\n\
                 Input tokens:  {inp} (${input_cost:.4})\n\
                 Output tokens: {out} (${output_cost:.4})\n\
                 {cache_line}\n\
                 Total cost:    ${total:.4}\n",
                turns = stats.turn_count,
                inp = stats.input_tokens,
                out = stats.output_tokens,
            );
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /tasks ────────────────────────────────────────────
        "/tasks" => {
            let tasks = archon_tools::task_manager::TASK_MANAGER.list_tasks();
            if tasks.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta("\nNo background tasks.\n".into()))
                    .await;
            } else {
                let mut out = format!("\n{} background tasks:\n", tasks.len());
                for t in &tasks {
                    out.push_str(&format!("  {} [{}] {}\n", &t.id, t.status, t.description));
                }
                let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
            }
            true
        }
        // ── /release-notes ────────────────────────────────────
        "/release-notes" => {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(
                    "\nArchon CLI v0.1.0 (Phase 3)\n\n\
                 - 33 tasks implemented across 7 batches\n\
                 - TUI with markdown rendering, syntax highlighting, vim mode\n\
                 - MCP stdio + HTTP transports with lifecycle management\n\
                 - Memory graph with HNSW vector search\n\
                 - 46 slash commands, hook system, config hot-reload\n\
                 - Background sessions, task tools, worktree support\n\
                 - Permission model with 6 modes\n\
                 - Print mode (-p) for scripting\n\
                 - /btw side questions with parallel API calls\n\n\
                 Full changelog: https://github.com/archon-cli/archon/releases\n"
                        .into(),
                ))
                .await;
            true
        }
        // ── /reload ───────────────────────────────────────────
        "/reload" => {
            // Force config reload from disk
            match archon_core::config_watcher::force_reload(
                std::slice::from_ref(&ctx.config_path),
                &archon_core::config::ArchonConfig::default(),
            ) {
                Ok((_new_cfg, changed)) => {
                    if changed.is_empty() {
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(
                                "\nConfig reloaded. No changes detected.\n".into(),
                            ))
                            .await;
                    } else {
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nConfig reloaded. Changed: {}\n",
                                changed.join(", ")
                            )))
                            .await;
                    }
                }
                Err(e) => {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Config reload failed: {e}")))
                        .await;
                }
            }
            true
        }
        // ── /logout ───────────────────────────────────────────
        "/logout" => {
            // Clear OAuth credentials file
            let cred_path = dirs::home_dir()
                .unwrap_or_default()
                .join(".archon")
                .join(".credentials.json");
            if cred_path.exists() {
                match std::fs::remove_file(&cred_path) {
                    Ok(()) => {
                        let _ = tui_tx.send(TuiEvent::TextDelta(
                            "\nLogged out. Credentials cleared.\nRestart and run /login to re-authenticate.\n".into()
                        )).await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Failed to clear credentials: {e}")))
                            .await;
                    }
                }
            } else {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(
                        "\nNo stored credentials found. Using API key auth.\n".into(),
                    ))
                    .await;
            }
            true
        }
        // ── /help ──────────────────────────────────────────────
        s if s == "/help" || s.starts_with("/help ") => {
            let arg = s.strip_prefix("/help").unwrap_or("").trim();
            if arg.is_empty() {
                let mut help_text = "\n\
                    Core commands:\n\
                    /model <name>        - Switch model (opus, sonnet, haiku, or full name)\n\
                    /fast                - Toggle fast mode\n\
                    /effort <level>      - Set effort (high, medium, low)\n\
                    /thinking on|off     - Show/hide thinking output\n\
                    /compact             - Trigger context compaction\n\
                    /clear               - Clear conversation history\n\
                    /status              - Show current session info\n\
                    /cost                - Show session cost breakdown\n\
                    /permissions [mode]  - Show/set permission mode (6 modes + aliases)\n\
                    /config [key] [val]  - List, get, or set runtime config values\n\
                    /memory [subcmd]     - List, search, or clear memories\n\
                    /doctor              - Run diagnostics on all subsystems\n\
                    /export              - Export conversation as JSON\n\
                    /diff                - Show git diff --stat for the working directory\n\
                    /help                - Show this help\n\
                    /help <command>      - Show detailed help for a command\n\n\
                    Extended commands:\n"
                    .to_string();
                let skill_help = ctx.skill_registry.format_help();
                help_text.push_str(&skill_help);
                let _ = tui_tx.send(TuiEvent::TextDelta(help_text)).await;
            } else {
                // Strip leading '/' from the argument if present
                let name = arg.strip_prefix('/').unwrap_or(arg);
                if let Some(detail) = ctx.skill_registry.format_skill_help(name) {
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta(format!("\n{detail}\n")))
                        .await;
                } else {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Unknown command: /{name}")))
                        .await;
                }
            }
            true
        }
        // ── /rename ─────────────────────────────────────────────
        s if s.starts_with("/rename") => {
            let name_arg = s.strip_prefix("/rename").unwrap_or("").trim();
            if name_arg.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::Error("Usage: /rename <name>".into()))
                    .await;
            } else {
                let db_path = archon_session::storage::default_db_path();
                match archon_session::storage::SessionStore::open(&db_path) {
                    Ok(store) => {
                        match archon_session::naming::set_session_name(
                            &store,
                            &ctx.session_id,
                            name_arg,
                        ) {
                            Ok(()) => {
                                let _ = tui_tx
                                    .send(TuiEvent::SessionRenamed(name_arg.to_string()))
                                    .await;
                                let _ = tui_tx
                                    .send(TuiEvent::TextDelta(format!(
                                        "\nSession renamed to: {name_arg}\n"
                                    )))
                                    .await;
                            }
                            Err(e) => {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!("Rename failed: {e}")))
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Session store error: {e}")))
                            .await;
                    }
                }
            }
            true
        }
        // ── /resume ─────────────────────────────────────────────
        s if s.starts_with("/resume") => {
            let arg = s.strip_prefix("/resume").unwrap_or("").trim();
            let db_path = archon_session::storage::default_db_path();
            match archon_session::storage::SessionStore::open(&db_path) {
                Ok(store) => {
                    if arg.is_empty() {
                        // Show interactive session picker
                        let query = archon_session::search::SessionSearchQuery::default();
                        match archon_session::search::search_sessions(&store, &query) {
                            Ok(results) => {
                                if results.is_empty() {
                                    let _ = tui_tx
                                        .send(TuiEvent::TextDelta(
                                            "\nNo previous sessions found.\n".into(),
                                        ))
                                        .await;
                                } else {
                                    let entries: Vec<archon_tui::app::SessionPickerEntry> = results
                                        .iter()
                                        .map(|m| archon_tui::app::SessionPickerEntry {
                                            id: m.id.clone(),
                                            name: m.name.clone().unwrap_or_default(),
                                            turns: m.message_count / 2,
                                            cost: m.total_cost,
                                            last_active: m.last_active.chars().take(10).collect(),
                                        })
                                        .collect();
                                    let _ = tui_tx.send(TuiEvent::ShowSessionPicker(entries)).await;
                                }
                            }
                            Err(e) => {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!("Search failed: {e}")))
                                    .await;
                            }
                        }
                    } else {
                        // Try to resolve by name or ID prefix
                        match archon_session::naming::resolve_by_name(&store, arg) {
                            Ok(Some(meta)) => {
                                let _ = tui_tx
                                    .send(TuiEvent::TextDelta(format!(
                                        "\nSession found: {}\nRestart with: archon --resume {}\n",
                                        meta.id, meta.id
                                    )))
                                    .await;
                            }
                            Ok(None) => {
                                let _ = tui_tx
                                    .send(TuiEvent::TextDelta(format!(
                                        "\nNo session matching '{arg}'. Use /sessions to list.\n"
                                    )))
                                    .await;
                            }
                            Err(e) => {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!("Lookup failed: {e}")))
                                    .await;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Session store error: {e}")))
                        .await;
                }
            }
            true
        }
        // ── /mcp (MCP server manager overlay) ─────────────────
        "/mcp" => {
            let info = ctx.mcp_manager.get_server_info().await;
            let mut entries: Vec<archon_tui::app::McpServerEntry> = Vec::new();
            for (name, state, disabled) in info {
                let state_str = if disabled {
                    "disabled"
                } else {
                    match state {
                        archon_mcp::types::ServerState::Ready => "ready",
                        archon_mcp::types::ServerState::Starting
                        | archon_mcp::types::ServerState::Restarting => "starting",
                        archon_mcp::types::ServerState::Crashed => "crashed",
                        archon_mcp::types::ServerState::Stopped => "stopped",
                    }
                };
                let tools = if state_str == "ready" {
                    ctx.mcp_manager.list_tools_for(&name).await
                } else {
                    Vec::new()
                };
                entries.push(archon_tui::app::McpServerEntry {
                    name: name.clone(),
                    state: state_str.to_string(),
                    tool_count: tools.len(),
                    disabled,
                    tools,
                });
            }
            let _ = tui_tx.send(TuiEvent::ShowMcpManager(entries)).await;
            true
        }
        // ── /fork (branch conversation) ────────────────────────
        s if s == "/fork" || s.starts_with("/fork ") => {
            let name_arg = s.strip_prefix("/fork").unwrap_or("").trim();
            let db_path = archon_session::storage::default_db_path();
            match archon_session::storage::SessionStore::open(&db_path) {
                Ok(store) => {
                    let fork_name = if name_arg.is_empty() {
                        None
                    } else {
                        Some(name_arg)
                    };
                    match archon_session::fork::fork_session(&store, &ctx.session_id, fork_name) {
                        Ok(new_id) => {
                            let _ = tui_tx.send(TuiEvent::TextDelta(
                                format!("\nConversation forked as: {new_id}\nResume with: archon --resume {new_id}\nOriginal session: {}\n", ctx.session_id)
                            )).await;
                        }
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("Fork failed: {e}")))
                                .await;
                        }
                    }
                }
                Err(e) => {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Session store error: {e}")))
                        .await;
                }
            }
            true
        }
        // ── /checkpoint list | /checkpoint restore <file> ──────
        s if s == "/checkpoint" || s.starts_with("/checkpoint ") => {
            let arg = s.strip_prefix("/checkpoint").unwrap_or("").trim();
            let ckpt_path = dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("archon")
                .join("checkpoints.db");
            if arg == "list" || arg.is_empty() {
                match archon_session::checkpoint::CheckpointStore::open(&ckpt_path) {
                    Ok(store) => match store.list_modified(&ctx.session_id) {
                        Ok(snapshots) if snapshots.is_empty() => {
                            let _ = tui_tx
                                .send(TuiEvent::TextDelta(
                                    "\nNo checkpoints for this session.\n".into(),
                                ))
                                .await;
                        }
                        Ok(snapshots) => {
                            let mut out = String::from("\nCheckpoints:\n");
                            for s in &snapshots {
                                out.push_str(&format!(
                                    "  turn {} | {} | {} | {}\n",
                                    s.turn_number, s.tool_name, s.file_path, s.timestamp
                                ));
                            }
                            let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
                        }
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("Checkpoint list error: {e}")))
                                .await;
                        }
                    },
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Checkpoint store error: {e}")))
                            .await;
                    }
                }
            } else if let Some(file_path) = arg.strip_prefix("restore").map(|s| s.trim()) {
                if file_path.is_empty() {
                    let _ = tui_tx
                        .send(TuiEvent::Error(
                            "Usage: /checkpoint restore <file_path>".into(),
                        ))
                        .await;
                } else {
                    match archon_session::checkpoint::CheckpointStore::open(&ckpt_path) {
                        Ok(store) => match store.restore(&ctx.session_id, file_path) {
                            Ok(()) => {
                                let _ = tui_tx
                                    .send(TuiEvent::TextDelta(format!("\nRestored: {file_path}\n")))
                                    .await;
                            }
                            Err(e) => {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!("Restore failed: {e}")))
                                    .await;
                            }
                        },
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("Checkpoint store error: {e}")))
                                .await;
                        }
                    }
                }
            } else {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(
                        "\nUsage: /checkpoint list | /checkpoint restore <file_path>\n".into(),
                    ))
                    .await;
            }
            true
        }
        // ── /add-dir ───────────────────────────────────────────
        s if s.starts_with("/add-dir") => {
            let path_arg = s.strip_prefix("/add-dir").unwrap_or("").trim();
            if path_arg.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::Error("Usage: /add-dir <path>".into()))
                    .await;
            } else {
                let path = std::path::PathBuf::from(path_arg);
                if path.is_dir() {
                    // Add to the shared extra directories list (visible to agent tool context)
                    ctx.extra_dirs.lock().await.push(path.clone());
                    let _ = tui_tx.send(TuiEvent::TextDelta(
                        format!("\nAdded '{}' to working directories for this session.\nFiles in this directory are now accessible.\n", path.display())
                    )).await;
                    tracing::info!(dir = %path.display(), "added working directory via /add-dir");
                } else {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Directory not found: {path_arg}")))
                        .await;
                }
            }
            true
        }
        // ── /color ─────────────────────────────────────────────
        s if s.starts_with("/color") => {
            let color_arg = s.strip_prefix("/color").unwrap_or("").trim();
            if color_arg.is_empty() {
                let _ = tui_tx.send(TuiEvent::TextDelta(
                    "\nAvailable accent colors: red, green, yellow, blue, magenta, cyan, white, default\n\
                     Usage: /color <name>\n".into()
                )).await;
            } else if let Some(color) = archon_tui::theme::parse_color(color_arg) {
                let _ = tui_tx.send(TuiEvent::SetAccentColor(color)).await;
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nAccent color set to '{color_arg}'.\n"
                    )))
                    .await;
            } else {
                let _ = tui_tx.send(TuiEvent::Error(
                    format!("Unknown color '{color_arg}'. Available: red, green, yellow, blue, magenta, cyan, white, default")
                )).await;
            }
            true
        }
        // ── /theme ─────────────────────────────────────────────
        s if s.starts_with("/theme") => {
            let theme_arg = s.strip_prefix("/theme").unwrap_or("").trim();
            if theme_arg.is_empty() {
                let names = archon_tui::theme::available_themes().join(", ");
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nAvailable themes: {names}\nUsage: /theme <name>\n"
                    )))
                    .await;
            } else if archon_tui::theme::theme_by_name(theme_arg).is_some() {
                let _ = tui_tx.send(TuiEvent::SetTheme(theme_arg.to_string())).await;
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nTheme set to '{theme_arg}'.\n"
                    )))
                    .await;
            } else {
                let names = archon_tui::theme::available_themes().join(", ");
                let _ = tui_tx
                    .send(TuiEvent::Error(format!(
                        "Unknown theme '{theme_arg}'. Available: {names}"
                    )))
                    .await;
            }
            true
        }
        // ── /recall ────────────────────────────────────────────
        s if s.starts_with("/recall") => {
            let query = s.strip_prefix("/recall").unwrap_or("").trim();
            if query.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::Error(
                        "Usage: /recall <query> — search memories by keyword".into(),
                    ))
                    .await;
            } else {
                // Search the memory graph
                let results = ctx.memory.recall_memories(query, 10);
                match results {
                    Ok(memories) => {
                        if memories.is_empty() {
                            let _ = tui_tx
                                .send(TuiEvent::TextDelta(format!(
                                    "\nNo memories found for '{query}'.\n"
                                )))
                                .await;
                        } else {
                            let mut out =
                                format!("\n{} memories matching '{query}':\n\n", memories.len());
                            for m in &memories {
                                let title = if m.title.is_empty() {
                                    "(untitled)"
                                } else {
                                    &m.title
                                };
                                let snippet: String = m.content.chars().take(100).collect();
                                let id_short = &m.id[..8.min(m.id.len())];
                                out.push_str(&format!(
                                    "  [{id_short}] {title}\n    {snippet}...\n\n"
                                ));
                            }
                            let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
                        }
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Memory search failed: {e}")))
                            .await;
                    }
                }
            }
            true
        }
        // ── /rules — list, edit, remove behavioral rules (CRIT-14 ITEM 4) ──
        s if s == "/rules" || s.starts_with("/rules ") => {
            let args_str = s.strip_prefix("/rules").unwrap_or("").trim();
            let engine = RulesEngine::new(ctx.memory.as_ref());
            if args_str.is_empty() || args_str == "list" {
                match engine.get_rules_sorted() {
                    Ok(rules) if rules.is_empty() => {
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta("\nNo behavioral rules.\n".into()))
                            .await;
                    }
                    Ok(rules) => {
                        let mut out = format!("\n{} behavioral rules:\n\n", rules.len());
                        for r in &rules {
                            let id_short = &r.id[..8.min(r.id.len())];
                            out.push_str(&format!(
                                "  [{id_short}] (score: {:.1}) {}\n",
                                r.score, r.text
                            ));
                        }
                        let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("rules list failed: {e}")))
                            .await;
                    }
                }
            } else if let Some(rest) = args_str.strip_prefix("edit ") {
                // /rules edit <id> <new text>
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                if parts.len() < 2 {
                    let _ = tui_tx
                        .send(TuiEvent::Error("Usage: /rules edit <id> <new text>".into()))
                        .await;
                } else {
                    let id_prefix = parts[0];
                    let new_text = parts[1];
                    // Resolve full ID from prefix
                    match engine.get_rules_sorted() {
                        Ok(rules) => {
                            if let Some(rule) = rules.iter().find(|r| r.id.starts_with(id_prefix)) {
                                match engine.update_rule(&rule.id, new_text) {
                                    Ok(()) => {
                                        let _ = tui_tx
                                            .send(TuiEvent::TextDelta(format!(
                                                "\nRule updated: {new_text}\n"
                                            )))
                                            .await;
                                    }
                                    Err(e) => {
                                        let _ = tui_tx
                                            .send(TuiEvent::Error(format!(
                                                "update_rule failed: {e}"
                                            )))
                                            .await;
                                    }
                                }
                            } else {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!(
                                        "No rule matching ID prefix '{id_prefix}'"
                                    )))
                                    .await;
                            }
                        }
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("rules lookup failed: {e}")))
                                .await;
                        }
                    }
                }
            } else if let Some(id_prefix) = args_str.strip_prefix("remove ") {
                let id_prefix = id_prefix.trim();
                match engine.get_rules_sorted() {
                    Ok(rules) => {
                        if let Some(rule) = rules.iter().find(|r| r.id.starts_with(id_prefix)) {
                            match engine.remove_rule(&rule.id) {
                                Ok(()) => {
                                    let _ = tui_tx
                                        .send(TuiEvent::TextDelta(format!(
                                            "\nRule removed: {}\n",
                                            rule.text
                                        )))
                                        .await;
                                }
                                Err(e) => {
                                    let _ = tui_tx
                                        .send(TuiEvent::Error(format!("remove_rule failed: {e}")))
                                        .await;
                                }
                            }
                        } else {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!(
                                    "No rule matching ID prefix '{id_prefix}'"
                                )))
                                .await;
                        }
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("rules lookup failed: {e}")))
                            .await;
                    }
                }
            } else {
                let _ = tui_tx
                    .send(TuiEvent::Error(
                        "Usage: /rules [list | edit <id> <text> | remove <id>]".into(),
                    ))
                    .await;
            }
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// /diff handler
// ---------------------------------------------------------------------------

pub(crate) async fn handle_diff_command(tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>, working_dir: &PathBuf) {
    let result = tokio::process::Command::new("git")
        .arg("diff")
        .arg("--stat")
        .current_dir(working_dir)
        .output()
        .await;

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !output.status.success() {
                if stderr.contains("not a git repository") {
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta("\nNot in a git repository.\n".into()))
                        .await;
                } else {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("git diff failed: {stderr}")))
                        .await;
                }
                return;
            }
            if stdout.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta("\nNo uncommitted changes.\n".into()))
                    .await;
            } else {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!("\n{stdout}")))
                    .await;
            }
        }
        Err(e) => {
            let _ = tui_tx
                .send(TuiEvent::Error(format!("Failed to run git: {e}")))
                .await;
        }
    }
}
