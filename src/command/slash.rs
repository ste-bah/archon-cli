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
    // TASK-AGS-807 snapshot-pattern builder. Pre-populates
    // `CommandContext::status_snapshot` (owned values, no locks) when
    // the primary command resolves to /status or its alias /info.
    // Sync CommandHandler::execute cannot await; the builder bridges
    // that gap here at the dispatch site where .await is legal.
    let mut __cmd_ctx = crate::command::context::build_command_context(
        input,
        tui_tx.clone(),
        ctx,
    )
    .await;
    let _ = ctx.dispatcher.dispatch(&mut __cmd_ctx, input);
    // TASK-AGS-808 effect-slot drain. Handlers that need to write to
    // async-guarded shared state (e.g. /model mutating
    // `model_override_shared`) stash a CommandEffect in
    // `pending_effect` synchronously; we consume it with `.take()`
    // here — where `.await` is legal — and apply the mutation via
    // `command::context::apply_effect`. Single-shot by construction.
    if let Some(effect) = __cmd_ctx.pending_effect.take() {
        crate::command::context::apply_effect(effect, ctx).await;
    }
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
        // Body migrated to src/command/model.rs (TASK-AGS-808).
        // Read side (no-args): ModelSnapshot populated by
        // build_command_context. Write side (with arg):
        // CommandEffect::SetModelOverride stored in
        // CommandContext::pending_effect; slash.rs post-dispatch
        // apply_effect awaits the mutex write on
        // slash_ctx.model_override_shared. Aliases: [m, switch-model].
        // Do not re-add the legacy arm — TUI-410 lesson.
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
        // Body migrated to src/command/context_cmd.rs (TASK-AGS-814).
        // Dispatcher at slash.rs:40-45 (PATH A hybrid) fires
        // ContextHandler via registry lookup. ContextSnapshot is
        // populated by build_command_context before dispatch
        // (single `session_stats.lock().await` in the builder —
        // SNAPSHOT-ONLY pattern). Aliases dropped from stub's
        // `["ctx"]` to `[]` because the legacy match arm only matched
        // `/context` literally — `/ctx` never worked for users. Do not
        // re-add the legacy arm — TUI-410 dead-code lesson.
        // ── /status ────────────────────────────────────────────
        // Body migrated to src/command/status.rs (TASK-AGS-807).
        // Dispatcher at slash.rs:35-41 (PATH A hybrid) fires StatusHandler
        // via alias-aware registry lookup. StatusSnapshot is populated by
        // build_command_context before dispatch. Aliases: [info].
        // Do not re-add the legacy arm — see TUI-410 lesson.
        // ── /cost ──────────────────────────────────────────────
        // Body migrated to src/command/cost.rs (TASK-AGS-809).
        // Dispatcher at slash.rs:40-55 (PATH A hybrid) fires CostHandler
        // via alias-aware registry lookup. CostSnapshot is populated by
        // build_command_context before dispatch (single mutex acquisition
        // on session_stats; cache_stats_line + hard_label pre-computed).
        // Aliases: [billing] only — spec wanted [usage, billing] but
        // `usage` is already a shipped primary (UsageHandler) so the
        // collision-free subset is all we apply. See cost.rs rustdoc
        // for the CONFIRM R-item. Do not re-add the legacy arm — see
        // TUI-410 lesson.
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
        // ── /memory: body migrated to src/command/memory.rs (AGS-817,
        //    DIRECT pattern). Dispatcher PATH A at slash.rs:40 fires
        //    MemoryHandler::execute via the registry BEFORE this arm;
        //    build_command_context populates `CommandContext::memory`
        //    UNCONDITIONALLY (mirrors AGS-815 session_id — Arc<dyn
        //    MemoryTrait> is cheap to clone). Arm deleted per TUI-410
        //    dead-code rule. Do NOT re-add — see TUI-410 lesson.
        //    ──────────────────────────────────────────────────────
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
        // TASK-AGS-806: body migrated to
        // `crate::command::task::TasksHandler` (registered as a
        // primary in registry.rs). Legacy match arm removed; the
        // dispatcher routes /tasks (and aliases todo/ps/jobs)
        // through the registry path.
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
        // ── /resume: body migrated to src/command/resume.rs (AGS-810,
        //    DIRECT pattern). Dispatcher PATH A at slash.rs:46 fires
        //    ResumeHandler::execute via the registry BEFORE this arm;
        //    aliases /continue and /open-session route there too. Arm
        //    deleted per TUI-410 dead-code rule. ──────────────────
        // ── /mcp: body migrated to src/command/mcp.rs (AGS-811,
        //    SNAPSHOT-ONLY pattern). Dispatcher PATH A at slash.rs:46
        //    fires McpHandler::execute via the registry BEFORE this
        //    arm; build_command_context awaits
        //    McpServerManager::get_server_info + list_tools_for at the
        //    dispatch site and threads an owned McpSnapshot through
        //    CommandContext. Arm deleted per TUI-410 dead-code rule.
        //    ──────────────────────────────────────────────────────
        // ── /fork: body migrated to src/command/fork.rs (AGS-815,
        //    DIRECT pattern). Dispatcher PATH A at slash.rs:46 fires
        //    ForkHandler::execute via the registry BEFORE this arm;
        //    build_command_context populates `CommandContext::session_id`
        //    unconditionally (DIRECT-pattern contract — not gated on
        //    the primary name, unlike the SNAPSHOT-ONLY fields) so the
        //    sync handler body can call
        //    `archon_session::fork::fork_session` without needing a
        //    per-command match arm in the builder. Arm deleted per
        //    TUI-410 dead-code rule. Do NOT re-add — see TUI-410 lesson.
        //    ──────────────────────────────────────────────────────
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
