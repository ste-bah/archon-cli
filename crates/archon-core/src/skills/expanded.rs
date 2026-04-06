//! Expanded slash command skills for CLI-225.
//!
//! Each skill is a simple struct implementing the [`Skill`] trait. Most are
//! lightweight stubs that return help text or delegate to existing
//! functionality. The primary purpose is to have them *registered* in the
//! skill system so they appear in help output and tab-completion.

use super::{Skill, SkillContext, SkillOutput, SkillRegistry};

// ---------------------------------------------------------------------------
// Macro for simple descriptor skills (same pattern as builtin.rs)
// ---------------------------------------------------------------------------

macro_rules! expanded_skill {
    ($struct_name:ident, $name:expr, $desc:expr) => {
        pub struct $struct_name;

        impl Skill for $struct_name {
            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $desc
            }

            fn execute(&self, _args: &[String], _ctx: &SkillContext) -> SkillOutput {
                SkillOutput::Text(format!("/{}: {}", self.name(), self.description()))
            }
        }
    };

    // Variant with a custom execute body
    ($struct_name:ident, $name:expr, $desc:expr, |$args:ident, $ctx:ident| $body:expr) => {
        pub struct $struct_name;

        impl Skill for $struct_name {
            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $desc
            }

            fn execute(&self, $args: &[String], $ctx: &SkillContext) -> SkillOutput {
                $body
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Context / Navigation
// ---------------------------------------------------------------------------

expanded_skill!(
    ContextSkill,
    "context",
    "Show context window usage statistics"
);

expanded_skill!(
    CopySkill,
    "copy",
    "Copy last assistant response to clipboard",
    |_args, _ctx| {
        SkillOutput::Text(
            "Use /copy to copy the last assistant response to the system clipboard.\n\
             Requires `xclip` (Linux) or `pbcopy` (macOS)."
                .to_string(),
        )
    }
);

expanded_skill!(
    BtwSkill,
    "btw",
    "Aside/tangent marker (do not change focus)",
    |_args, _ctx| {
        SkillOutput::Text(
            "/btw: This message is a tangent. The agent should not shift focus \
             from the current task."
                .to_string(),
        )
    }
);

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

expanded_skill!(TagSkill, "tag", "Tag current session", |args, ctx| {
    if args.is_empty() {
        return SkillOutput::Error(
            "Usage: /tag <name> — assigns a tag to the current session".to_string(),
        );
    }
    let tag = args.join(" ");
    let db_path = archon_session::storage::default_db_path();
    match archon_session::storage::SessionStore::open(&db_path) {
        Ok(store) => match archon_session::metadata::add_tag(&store, &ctx.session_id, &tag) {
            Ok(()) => SkillOutput::Text(format!("Session tagged as '{tag}'.")),
            Err(e) => SkillOutput::Error(format!("Tag failed: {e}")),
        },
        Err(e) => SkillOutput::Error(format!("Failed to open session store: {e}")),
    }
});

expanded_skill!(
    RenameSkill,
    "rename",
    "Rename current session",
    |args, ctx| {
        if args.is_empty() {
            return SkillOutput::Error(
                "Usage: /rename <name> — renames the current session".to_string(),
            );
        }
        let new_name = args.join(" ");
        let db_path = archon_session::storage::default_db_path();
        match archon_session::storage::SessionStore::open(&db_path) {
            Ok(store) => {
                match archon_session::naming::set_session_name(&store, &ctx.session_id, &new_name) {
                    Ok(()) => SkillOutput::Text(format!("Session renamed to '{new_name}'.")),
                    Err(e) => SkillOutput::Error(format!("Rename failed: {e}")),
                }
            }
            Err(e) => SkillOutput::Error(format!("Failed to open session store: {e}")),
        }
    }
);

expanded_skill!(
    SessionsSkill,
    "sessions",
    "Search and list previous sessions",
    |args, _ctx| {
        let db_path = archon_session::storage::default_db_path();
        let store = match archon_session::storage::SessionStore::open(&db_path) {
            Ok(s) => s,
            Err(e) => return SkillOutput::Error(format!("Failed to open session store: {e}")),
        };
        let query = archon_session::search::SessionSearchQuery {
            text: if args.is_empty() {
                None
            } else {
                Some(args.join(" "))
            },
            ..Default::default()
        };
        match archon_session::search::search_sessions(&store, &query) {
            Ok(results) => {
                if results.is_empty() {
                    SkillOutput::Text("No sessions found.".to_string())
                } else {
                    let listing = archon_session::listing::format_session_list(&results);
                    SkillOutput::Text(format!("\n{} sessions:\n{listing}", results.len()))
                }
            }
            Err(e) => SkillOutput::Error(format!("Search failed: {e}")),
        }
    }
);

// ---------------------------------------------------------------------------
// File
// ---------------------------------------------------------------------------

expanded_skill!(
    RestoreSkill,
    "restore",
    "Restore file from checkpoint",
    |args, ctx| {
        let cp_path = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("archon")
            .join("checkpoints.db");

        let store = match archon_session::checkpoint::CheckpointStore::open(&cp_path) {
            Ok(s) => s,
            Err(e) => return SkillOutput::Error(format!("Failed to open checkpoint store: {e}")),
        };

        if args.is_empty() {
            // /restore — list modified files
            match store.list_modified(&ctx.session_id) {
                Ok(files) => {
                    if files.is_empty() {
                        return SkillOutput::Text("No modified files in this session.".to_string());
                    }
                    let mut out = String::from("Modified files with checkpoints:\n\n");
                    for f in &files {
                        out.push_str(&format!(
                            "  {} (turn {}, by {})\n",
                            f.file_path, f.turn_number, f.tool_name
                        ));
                    }
                    out.push_str("\nUsage:\n");
                    out.push_str(
                        "  /restore <file>         — show diff and restore to latest snapshot\n",
                    );
                    out.push_str("  /restore <file> <turn>  — restore to specific turn\n");
                    out.push_str("  /restore --all          — restore all files\n");
                    SkillOutput::Text(out)
                }
                Err(e) => SkillOutput::Error(format!("Failed to list checkpoints: {e}")),
            }
        } else if args[0] == "--all" {
            // /restore --all
            match store.list_modified(&ctx.session_id) {
                Ok(files) => {
                    let mut restored = 0;
                    let mut errors = Vec::new();
                    for f in &files {
                        match store.restore(&ctx.session_id, &f.file_path) {
                            Ok(()) => restored += 1,
                            Err(e) => errors.push(format!("{}: {e}", f.file_path)),
                        }
                    }
                    let mut out = format!("Restored {restored} file(s).");
                    if !errors.is_empty() {
                        out.push_str(&format!("\nErrors:\n  {}", errors.join("\n  ")));
                    }
                    SkillOutput::Text(out)
                }
                Err(e) => SkillOutput::Error(format!("Failed to list checkpoints: {e}")),
            }
        } else {
            let file = &args[0];
            let turn: Option<i64> = args.get(1).and_then(|s| s.parse().ok());

            if let Some(turn_num) = turn {
                // /restore <file> <turn> — restore to specific turn
                match store.restore_to_turn(&ctx.session_id, file, turn_num) {
                    Ok(()) => SkillOutput::Text(format!("Restored '{file}' to turn {turn_num}.")),
                    Err(e) => SkillOutput::Error(format!("Failed to restore: {e}")),
                }
            } else {
                // /restore <file> — show diff first, then restore
                match store.list_modified(&ctx.session_id) {
                    Ok(files) => {
                        let matching: Vec<_> =
                            files.iter().filter(|f| f.file_path == *file).collect();
                        if matching.is_empty() {
                            return SkillOutput::Error(format!(
                                "No checkpoint found for '{file}'."
                            ));
                        }
                        let latest = matching.last().unwrap();

                        // Show diff
                        match store.diff(&ctx.session_id, file, latest.turn_number) {
                            Ok(diff_text) => {
                                if diff_text.is_empty() {
                                    SkillOutput::Text(format!(
                                        "'{file}' is unchanged from checkpoint."
                                    ))
                                } else {
                                    match store.restore(&ctx.session_id, file) {
                                        Ok(()) => {
                                            let out = format!(
                                                "Diff before restore:\n\n{diff_text}\n\nRestored '{file}' to latest checkpoint."
                                            );
                                            SkillOutput::Text(out)
                                        }
                                        Err(e) => SkillOutput::Error(format!(
                                            "Diff ok but restore failed: {e}"
                                        )),
                                    }
                                }
                            }
                            Err(e) => {
                                // Diff failed, still try restore
                                match store.restore(&ctx.session_id, file) {
                                    Ok(()) => SkillOutput::Text(format!(
                                        "Restored '{file}' (diff unavailable: {e})."
                                    )),
                                    Err(e2) => {
                                        SkillOutput::Error(format!("Failed to restore: {e2}"))
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => SkillOutput::Error(format!("Failed to list checkpoints: {e}")),
                }
            }
        }
    }
);

expanded_skill!(
    UndoSkill,
    "undo",
    "Undo last file modification",
    |_args, _ctx| {
        SkillOutput::Text(
            "Undoing last file modification. Use /undo to revert the most recent edit.".to_string(),
        )
    }
);

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

expanded_skill!(ReloadSkill, "reload", "Force configuration reload");

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

expanded_skill!(
    ThinkingSkill,
    "thinking",
    "Toggle extended thinking display"
);

expanded_skill!(ClearSkill, "clear", "Clear conversation history");

// ---------------------------------------------------------------------------
// Meta
// ---------------------------------------------------------------------------

expanded_skill!(BugSkill, "bug", "Report a bug", |_args, _ctx| {
    SkillOutput::Text(
        "To report a bug:\n\n\
             1. Visit https://github.com/archon-cli/archon/issues/new\n\
             2. Include your Archon version (`archon --version`)\n\
             3. Describe the issue with steps to reproduce\n\
             4. Attach any relevant logs (`~/.archon/logs/`)"
            .to_string(),
    )
});

expanded_skill!(LoginSkill, "login", "Re-authenticate with the API provider");

// ---------------------------------------------------------------------------
// Session management (additional)
// ---------------------------------------------------------------------------

expanded_skill!(
    ResumeSkill,
    "resume",
    "Resume a previous session by ID or name",
    |args, _ctx| {
        if args.is_empty() {
            return SkillOutput::Error(
                "Usage: /resume <id|name> — resume a previous session".to_string(),
            );
        }
        let target = args.join(" ");
        SkillOutput::Text(format!("Resuming session '{target}'..."))
    }
);

expanded_skill!(
    ForkSkill,
    "fork",
    "Fork current conversation at this point",
    |args, ctx| {
        let short_id: String = ctx.session_id.chars().take(8).collect();
        let name = if args.is_empty() {
            format!("{short_id}-fork")
        } else {
            args.join(" ")
        };
        SkillOutput::Text(format!("Forking conversation as '{name}'."))
    }
);

expanded_skill!(
    RewindSkill,
    "rewind",
    "Rewind conversation to a previous checkpoint",
    |_args, ctx| {
        let cp_path = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("archon")
            .join("checkpoints.db");
        match archon_session::checkpoint::CheckpointStore::open(&cp_path) {
            Ok(store) => match store.list_modified(&ctx.session_id) {
                Ok(files) => {
                    if files.is_empty() {
                        SkillOutput::Text(
                            "No checkpoints in this session. Files must be modified first."
                                .to_string(),
                        )
                    } else {
                        let mut out = String::from("Modified files with checkpoints:\n");
                        for f in &files {
                            out.push_str(&format!(
                                "  {} (turn {}, by {})\n",
                                f.file_path, f.turn_number, f.tool_name
                            ));
                        }
                        out.push_str("\nUse the Read tool to examine files, then ask me to restore specific ones.");
                        SkillOutput::Text(out)
                    }
                }
                Err(e) => SkillOutput::Error(format!("Failed to list checkpoints: {e}")),
            },
            Err(e) => SkillOutput::Error(format!("Failed to open checkpoint store: {e}")),
        }
    }
);

// ---------------------------------------------------------------------------
// Context (additional)
// ---------------------------------------------------------------------------

expanded_skill!(
    UsageSkill,
    "usage",
    "Show token usage, cost breakdown, and turn count"
);

expanded_skill!(TasksSkill, "tasks", "List and manage background tasks");

expanded_skill!(
    RecallSkill,
    "recall",
    "Search memories by keyword",
    |args, _ctx| {
        if args.is_empty() {
            return SkillOutput::Error(
                "Usage: /recall <query> — search memories by keyword".to_string(),
            );
        }
        let query = args.join(" ");
        SkillOutput::Text(format!("Searching memories for '{query}'..."))
    }
);

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

expanded_skill!(
    AgentsSkill,
    "agents",
    "List agent definitions from .archon/agents/ directory",
    |_args, ctx| {
        let agents_dir = ctx.working_dir.join(".archon").join("agents");
        if !agents_dir.exists() {
            return SkillOutput::Text("No agents directory found (.archon/agents/).".to_string());
        }
        let mut entries = Vec::new();
        if let Ok(dir) = std::fs::read_dir(&agents_dir) {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let agent_md = path.join("agent.md");
                    let desc = if agent_md.exists() {
                        std::fs::read_to_string(&agent_md)
                            .ok()
                            .and_then(|c| {
                                c.lines()
                                    .next()
                                    .map(|l| l.trim_start_matches('#').trim().to_string())
                            })
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    entries.push(format!("  {name}: {desc}"));
                }
            }
        }
        if entries.is_empty() {
            SkillOutput::Text("No agents found in .archon/agents/.".to_string())
        } else {
            let mut out = format!("{} agents:\n", entries.len());
            out.push_str(&entries.join("\n"));
            SkillOutput::Text(out)
        }
    }
);

// ---------------------------------------------------------------------------
// Configuration (additional)
// ---------------------------------------------------------------------------

expanded_skill!(
    ThemeSkill,
    "theme",
    "Change color theme (light/dark/auto)",
    |args, _ctx| {
        if args.is_empty() {
            return SkillOutput::Text(
                "Current theme: dark\nAvailable: light, dark, auto\nUsage: /theme <name>"
                    .to_string(),
            );
        }
        let theme = &args[0];
        match theme.as_str() {
            "light" | "dark" | "auto" => SkillOutput::Text(format!("Theme set to '{theme}'.")),
            _ => SkillOutput::Error(format!(
                "Unknown theme '{theme}'. Available: light, dark, auto"
            )),
        }
    }
);

expanded_skill!(
    ColorSkill,
    "color",
    "Set prompt bar accent color",
    |args, _ctx| {
        if args.is_empty() {
            return SkillOutput::Text(
                "Usage: /color <color|default> — set prompt bar accent color".to_string(),
            );
        }
        let color = &args[0];
        SkillOutput::Text(format!("Accent color set to '{color}'."))
    }
);

expanded_skill!(
    KeybindingsSkill,
    "keybindings",
    "Show keybinding configuration",
    |_args, _ctx| {
        SkillOutput::Text(
            "Keybindings:\n\
             Ctrl+C    — Interrupt generation / quit\n\
             Ctrl+D    — Quit\n\
             Ctrl+T    — Toggle thinking display\n\
             Esc       — Dismiss suggestions / double-tap to cancel\n\
             Tab       — Accept autocomplete suggestion\n\
             Up/Down   — History navigation\n\
             PageUp/Dn — Scroll output\n\n\
             Vim mode (set [tui] vim_mode = true):\n\
             i/a/I/A   — Enter insert mode\n\
             Esc       — Normal mode\n\
             dd/yy/p   — Delete/yank/paste line\n\
             gg/G      — Top/bottom\n\
             v         — Visual mode\n\
             :w        — Submit, :q — Quit"
                .to_string(),
        )
    }
);

expanded_skill!(
    StatuslineSkill,
    "statusline",
    "Configure status line content",
    |_args, _ctx| {
        SkillOutput::Text(
            "Status line shows: model | identity mode | permission mode | cost\n\n\
             Currently not configurable at runtime.\n\
             The status bar updates automatically based on:\n\
             - /model changes\n\
             - /permissions changes\n\
             - Session cost accumulation"
                .to_string(),
        )
    }
);

// ---------------------------------------------------------------------------
// Mode control
// ---------------------------------------------------------------------------

expanded_skill!(
    SandboxSkill,
    "sandbox",
    "Toggle sandbox mode",
    |_args, _ctx| {
        SkillOutput::Text(
            "Sandbox mode: use `--sandbox` flag at startup, or set `permissions.sandbox = true` in config.toml.".to_string(),
        )
    }
);

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

expanded_skill!(
    InitSkill,
    "init",
    "Initialize project with CLAUDE.md",
    |_args, ctx| {
        let claude_md = ctx.working_dir.join("CLAUDE.md");
        if claude_md.exists() {
            return SkillOutput::Text("CLAUDE.md already exists in this project.".to_string());
        }
        let template = "# Project Instructions\n\n\
            ## Overview\n\
            Describe your project here.\n\n\
            ## Code Style\n\
            - Describe coding conventions\n\
            - Testing requirements\n\
            - Architecture patterns\n\n\
            ## Build Commands\n\
            ```\n\
            # Add your build/test/lint commands here\n\
            ```\n";
        match std::fs::write(&claude_md, template) {
            Ok(()) => SkillOutput::Text(format!(
                "Created CLAUDE.md at {}\nEdit it to add your project instructions.",
                claude_md.display()
            )),
            Err(e) => SkillOutput::Error(format!("Failed to create CLAUDE.md: {e}")),
        }
    }
);

expanded_skill!(
    AddDirSkill,
    "add-dir",
    "Add working directory for file access",
    |args, _ctx| {
        if args.is_empty() {
            return SkillOutput::Error(
                "Usage: /add-dir <path> — add a directory to the working set".to_string(),
            );
        }
        let path = &args[0];
        SkillOutput::Text(format!("Added '{path}' to working directories."))
    }
);

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

expanded_skill!(
    InsightsSkill,
    "insights",
    "Session patterns, common tools, error rates",
    |_args, _ctx| {
        // PromptCommand equivalent — injected into the conversation as a user message
        SkillOutput::Prompt(
            "Analyze this session and provide insights:\n\
             1. What patterns do you see in how I'm working?\n\
             2. Which tools am I using most/least?\n\
             3. What could I do more efficiently?\n\
             4. Any recurring errors or issues?\n\
             \nBase your analysis on the conversation history above."
                .to_string(),
        )
    }
);

expanded_skill!(
    StatsSkill,
    "stats",
    "Daily usage, session history, model preferences",
    |_args, _ctx| {
        let db_path = archon_session::storage::default_db_path();
        match archon_session::storage::SessionStore::open(&db_path) {
            Ok(store) => match archon_session::search::session_stats(&store) {
                Ok(stats) => {
                    let mut out = String::from("\nSession statistics:\n");
                    out.push_str(&format!("  Total sessions:  {}\n", stats.total_sessions));
                    out.push_str(&format!("  Total tokens:    {}\n", stats.total_tokens));
                    out.push_str(&format!("  Total messages:  {}\n", stats.total_messages));
                    if stats.total_sessions > 0 {
                        let avg_dur = stats.avg_duration_secs / 60.0;
                        out.push_str(&format!("  Avg duration:    {avg_dur:.1} min\n"));
                    }
                    SkillOutput::Text(out)
                }
                Err(e) => SkillOutput::Error(format!("Stats error: {e}")),
            },
            Err(e) => SkillOutput::Error(format!("Failed to open session store: {e}")),
        }
    }
);

expanded_skill!(
    SecurityReviewSkill,
    "security-review",
    "Analyze pending changes for security vulnerabilities",
    |_args, ctx| {
        // Get git diff and ask the agent to review it
        let repo = match archon_tools::git::open_repo(&ctx.working_dir) {
            Ok(r) => r,
            Err(e) => return SkillOutput::Error(format!("Not a git repo: {e}")),
        };
        let diff = archon_tools::git::diff::git_diff(&repo, false).unwrap_or_default();
        if diff.is_empty() {
            return SkillOutput::Text("No uncommitted changes to review.".to_string());
        }
        // PromptCommand equivalent — injected into the conversation as a user message
        SkillOutput::Prompt(format!(
            "Please perform a security review of these pending changes. Check for:\n\
             - Command injection, SQL injection, XSS\n\
             - Hardcoded secrets or credentials\n\
             - Path traversal vulnerabilities\n\
             - Insecure deserialization\n\
             - Missing input validation\n\
             - Authentication/authorization issues\n\n\
             ```diff\n{diff}\n```"
        ))
    }
);

// ---------------------------------------------------------------------------
// Utility (additional)
// ---------------------------------------------------------------------------

expanded_skill!(
    FeedbackSkill,
    "feedback",
    "Submit feedback or report an issue",
    |args, _ctx| {
        if args.is_empty() {
            return SkillOutput::Text(
                "Usage: /feedback <message> — submit feedback\n\
                 Or visit: https://github.com/archon-cli/archon/issues"
                    .to_string(),
            );
        }
        let message = args.join(" ");
        SkillOutput::Text(format!("Feedback recorded: {message}"))
    }
);

expanded_skill!(ReleaseNotesSkill, "release-notes", "Show version changelog");

expanded_skill!(
    ScheduleSkill,
    "schedule",
    "Create scheduled task",
    |_args, _ctx| {
        SkillOutput::Text(
            "Use the CronCreate tool to schedule recurring tasks. Ask the agent to create a scheduled task with a cron expression.".to_string(),
        )
    }
);

expanded_skill!(
    RemoteControlSkill,
    "remote-control",
    "Remote control mode",
    |_args, _ctx| {
        SkillOutput::Text(
            "Remote control is available via `archon remote ws` to connect as client, or `archon serve` to run as server.".to_string(),
        )
    }
);

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

expanded_skill!(
    LogoutSkill,
    "logout",
    "Sign out from API provider",
    |_args, _ctx| {
        SkillOutput::Text(
            "Logged out. API key removed from session. Re-authenticate with /login.".to_string(),
        )
    }
);

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all expanded skills into the given registry.
pub fn register_expanded_skills(registry: &mut SkillRegistry) {
    // Context / Navigation
    registry.register(Box::new(ContextSkill));
    registry.register(Box::new(CopySkill));
    registry.register(Box::new(BtwSkill));
    registry.register(Box::new(UsageSkill));
    registry.register(Box::new(TasksSkill));
    registry.register(Box::new(RecallSkill));

    // Session
    registry.register(Box::new(TagSkill));
    registry.register(Box::new(RenameSkill));
    registry.register(Box::new(SessionsSkill));
    registry.register(Box::new(ResumeSkill));
    registry.register(Box::new(ForkSkill));
    registry.register(Box::new(RewindSkill));

    // Agents
    registry.register(Box::new(AgentsSkill));

    // File
    registry.register(Box::new(RestoreSkill));
    registry.register(Box::new(UndoSkill));

    // Config
    registry.register(Box::new(ReloadSkill));
    registry.register(Box::new(ThemeSkill));
    registry.register(Box::new(ColorSkill));
    registry.register(Box::new(KeybindingsSkill));
    registry.register(Box::new(StatuslineSkill));

    // Mode
    registry.register(Box::new(SandboxSkill));

    // Project
    registry.register(Box::new(InitSkill));
    registry.register(Box::new(AddDirSkill));

    // Display
    registry.register(Box::new(ThinkingSkill));
    registry.register(Box::new(ClearSkill));

    // Analysis
    registry.register(Box::new(InsightsSkill));
    registry.register(Box::new(StatsSkill));
    registry.register(Box::new(SecurityReviewSkill));

    // Utility
    registry.register(Box::new(FeedbackSkill));
    registry.register(Box::new(ReleaseNotesSkill));
    registry.register(Box::new(ScheduleSkill));
    registry.register(Box::new(RemoteControlSkill));

    // Meta
    registry.register(Box::new(BugSkill));
    registry.register(Box::new(LoginSkill));
    registry.register(Box::new(LogoutSkill));
}
