//! `archon reasoning` and `archon briefing` CLI handlers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::cli_args::{BriefingAction, ReasoningAction, ReasoningCostAction};

pub(crate) async fn handle_reasoning_command(
    action: &ReasoningAction,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    let root = reasoning_quality_root()?;
    let output = match action {
        ReasoningAction::Status => render_status(config, &root),
        ReasoningAction::Inspect {
            session_id,
            analyzer,
        } => render_inspect(&root, session_id, analyzer),
        ReasoningAction::Backfill {
            sessions,
            emit_world_rows,
            include_llm,
        } => crate::command::reasoning_backfill::render_backfill(
            &root,
            config,
            *sessions,
            *emit_world_rows,
            *include_llm,
        ),
        ReasoningAction::Claims { session_id } => render_claims(&root, session_id),
        ReasoningAction::Patterns => render_patterns(config, &root),
        ReasoningAction::Cost { action } => match action {
            ReasoningCostAction::Status => render_cost_status(config, &root),
        },
        ReasoningAction::ReplayDeadLetter { bridge } => {
            render_replay_dead_letter(&root, bridge.as_deref())
        }
        ReasoningAction::ShadowReport => render_shadow_report(config),
        ReasoningAction::SampleLabel { session_id, turn } => {
            crate::command::reasoning_label::render_sample_label(&root, session_id, *turn)
        }
        ReasoningAction::Migrate {
            to_version,
            dry_run,
        } => render_migrate(&root, *to_version, *dry_run),
        ReasoningAction::FixtureAudit => render_fixture_audit(config),
    }?;
    println!("{output}");
    Ok(())
}

pub(crate) async fn handle_briefing_command(
    action: &BriefingAction,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    let output = match action {
        BriefingAction::Preview { task } => render_briefing_preview(config, task.as_deref())?,
    };
    println!("{output}");
    Ok(())
}

fn render_status(config: &archon_core::config::ArchonConfig, root: &Path) -> Result<String> {
    let count = open_store(root)
        .and_then(|store| store.count_events())
        .unwrap_or(0);
    let rq = &config.learning.reasoning_quality;
    Ok(format!(
        "Reasoning Quality Status\n\
         ========================\n\
         Enabled: {}\n\
         Inline events: {}\n\
         Stored events: {}\n\
         Shadow mode days: {}\n\
         Max claims/turn: {}\n\
         Store raw text: {}\n\
         LLM critic: {} ({})\n\
         Critic cloud flow: policy-gated\n\
         Store: {}\n\
         Dead letters: {}",
        rq.enabled,
        rq.emit_inline_events,
        count,
        rq.shadow_mode_days,
        rq.max_claims_per_turn,
        rq.store_raw_text,
        rq.critic.allow_llm,
        rq.critic.mode,
        root.display(),
        dead_letter_count(root)
    ))
}

fn render_inspect(root: &Path, session_id: &str, analyzer: &str) -> Result<String> {
    let events = open_store(root)?.events_for_session(session_id)?;
    let mut counts = BTreeMap::new();
    for event in &events {
        *counts
            .entry(format!("{:?}", event.event_kind))
            .or_insert(0usize) += 1;
    }
    Ok(format!(
        "Reasoning Inspect\n\
         =================\n\
         Session: {session_id}\n\
         Analyzer: {analyzer}\n\
         Events: {}\n\
         Counts: {}\n\
         Summary-only retrospective dedup: enabled",
        events.len(),
        serde_json::to_string(&counts)?
    ))
}

fn render_claims(root: &Path, session_id: &str) -> Result<String> {
    let events = open_store(root)?.events_for_session(session_id)?;
    let mut out = format!("Reasoning Claims\n================\nSession: {session_id}\n");
    for event in events {
        out.push_str(&format!(
            "- turn {} | {:?} | {:?} | {} | {}\n",
            event.turn_number,
            event.event_kind,
            event.verification_state,
            event.claim_id,
            event.canonical_text
        ));
    }
    Ok(out)
}

fn render_patterns(config: &archon_core::config::ArchonConfig, root: &Path) -> Result<String> {
    let store = open_store(root)?;
    let events = store.recent_events(1_000)?;
    let cfg = &config.learning.reasoning_quality.patterns;
    let patterns = archon_reasoning_quality::detect_repeated_patterns(
        &events,
        cfg.window_days,
        cfg.min_events,
        cfg.min_distinct_sessions,
        config.learning.reasoning_quality.shadow_mode_days > 0,
    );
    let mut out = format!(
        "Reasoning Patterns\n==================\nDetected repeated patterns: {}\n",
        patterns.len()
    );
    for pattern in patterns.iter().take(20) {
        out.push_str(&format!(
            "- {} {:?} {:?} entity={} events={} sessions={} shadow={}\n",
            pattern.pattern_id,
            pattern.event_kind,
            pattern.subject,
            pattern.entity_key,
            pattern.event_count,
            pattern.distinct_sessions,
            pattern.shadow
        ));
    }
    if patterns.is_empty() {
        out.push_str("No repeated reasoning failures meet the configured threshold.\n");
    }
    Ok(out)
}

fn render_cost_status(config: &archon_core::config::ArchonConfig, root: &Path) -> Result<String> {
    let budget = &config.learning.reasoning_quality.critic.budget;
    let usage = crate::runtime::reasoning_critic::read_usage_summary(root, None);
    Ok(format!(
        "Reasoning Critic Cost\n\
         =====================\n\
         Critic enabled: {}\n\
         Per-session token cap: {}\n\
         Daily USD cap: {:.2}\n\
         Weekly USD cap: {:.2}\n\
         Daily tokens: {}\n\
         Weekly tokens: {}\n\
         Daily estimated USD: {:.4}\n\
         Weekly estimated USD: {:.4}\n\
         Coverage: ledger-backed\n\
         Budget events: {}",
        config.learning.reasoning_quality.critic.allow_llm,
        budget.per_session_token_cap,
        budget.daily_usd_cap,
        budget.weekly_usd_cap,
        usage.daily_tokens,
        usage.weekly_tokens,
        usage.daily_usd,
        usage.weekly_usd,
        budget.emit_cost_events
    ))
}

fn render_replay_dead_letter(root: &Path, bridge: Option<&str>) -> Result<String> {
    let entries = read_dead_letters(root)?;
    let learning_db = open_learning_db().ok();
    let world_root = world_model_root().ok();
    let mut replayed = 0usize;
    let mut skipped = 0usize;
    for entry in entries {
        if bridge.is_some_and(|wanted| wanted != entry.bridge) {
            skipped += 1;
            continue;
        }
        let Some(event) = entry.event_json else {
            skipped += 1;
            continue;
        };
        match entry.bridge.as_str() {
            "learning_event" => crate::runtime::reasoning_quality::bridge_reasoning_events(
                std::slice::from_ref(&event),
                learning_db.as_ref(),
                root,
                None,
                false,
                false,
            ),
            "world_model" => crate::runtime::reasoning_quality::bridge_reasoning_events(
                std::slice::from_ref(&event),
                None,
                root,
                world_root.as_deref(),
                true,
                false,
            ),
            "self_trust" => crate::runtime::reasoning_quality::bridge_reasoning_events(
                std::slice::from_ref(&event),
                None,
                root,
                None,
                false,
                true,
            ),
            _ => {
                skipped += 1;
                continue;
            }
        }
        replayed += 1;
    }
    Ok(format!(
        "Reasoning Dead-Letter Replay\n\
         ============================\n\
         Bridge filter: {}\n\
         Replayed: {}\n\
         Skipped: {}",
        bridge.unwrap_or("all"),
        replayed,
        skipped
    ))
}

fn render_shadow_report(config: &archon_core::config::ArchonConfig) -> Result<String> {
    let eval = fixture_evaluation(config)?;
    Ok(format!(
        "Reasoning Shadow Report\n\
         =======================\n\
         Fixture count: {}\n\
         Fixture precision: {:.2}\n\
         Fixture recall: {:.2}\n\
         Claim-before-source precision: {:.2}\n\
         Operator labels: 0\n\
         Shadow exit: blocked until operator labels exist",
        eval.fixture_count,
        eval.claim_precision,
        eval.claim_recall,
        eval.claim_before_source_precision
    ))
}

fn render_migrate(root: &Path, to_version: u32, dry_run: bool) -> Result<String> {
    let store = open_store(root)?;
    store.record_schema_migration(to_version, dry_run)?;
    Ok(format!(
        "Reasoning Schema Migration\n\
         ==========================\n\
         Target version: {to_version}\n\
         Dry run: {dry_run}\n\
         Planned mutations: 1\n\
         Cozo-only safe migration: {}",
        if dry_run { "planned" } else { "recorded" }
    ))
}

fn render_fixture_audit(config: &archon_core::config::ArchonConfig) -> Result<String> {
    let fixtures = load_fixtures(config)?;
    let audit = archon_reasoning_quality::audit_labeled_turns(&fixtures);
    let eval = archon_reasoning_quality::evaluate_labeled_turns(&fixtures);
    Ok(format!(
        "Reasoning Fixture Audit\n\
         =======================\n\
         Fixtures: {}\n\
         Findings: {}\n\
         Gates pass: {}\n\
         Precision: {:.2}\n\
         Recall: {:.2}\n\
         Claim-before-source precision: {:.2}",
        fixtures.len(),
        audit.finding_count,
        audit.passed() && eval.gates_pass(),
        eval.claim_precision,
        eval.claim_recall,
        eval.claim_before_source_precision
    ))
}

fn render_briefing_preview(
    config: &archon_core::config::ArchonConfig,
    task: Option<&str>,
) -> Result<String> {
    let policy = archon_policy::load_effective_policy(&std::env::current_dir()?)?;
    let reasoning_root = reasoning_quality_root()?;
    let world_root = world_model_root()?;
    let learning_db = open_learning_db().ok();
    let body = crate::runtime::proactive_briefing::build_session_briefing(
        config,
        &policy,
        Some(&reasoning_root),
        learning_db.as_ref(),
        Some(&world_root),
        "preview",
        task,
    )
    .unwrap_or_else(|| "No proactive briefing items available.".to_string());
    Ok(format!(
        "Proactive Session Briefing Preview\n\
         =================================\n\
         Enabled: {}\n\
         Task hint: {}\n\
         Includes: memory={}, reasoning_quality={}, pending_behaviour={}, world_model={}\n\
         Max items: {}\n\
         Max chars: {}\n\
         \n{}",
        config.learning.session_briefing.enabled,
        task.unwrap_or("not provided"),
        config.learning.session_briefing.include_memory,
        config.learning.session_briefing.include_reasoning_quality,
        config
            .learning
            .session_briefing
            .include_pending_behaviour_proposals,
        config.learning.session_briefing.include_world_model,
        config.learning.session_briefing.max_items,
        config.learning.session_briefing.max_chars,
        body
    ))
}

fn fixture_evaluation(
    config: &archon_core::config::ArchonConfig,
) -> Result<archon_reasoning_quality::FixtureEvaluation> {
    Ok(archon_reasoning_quality::evaluate_labeled_turns(
        &load_fixtures(config)?,
    ))
}

fn load_fixtures(
    config: &archon_core::config::ArchonConfig,
) -> Result<Vec<archon_reasoning_quality::LabeledTurnFixture>> {
    let dir = PathBuf::from(&config.learning.reasoning_quality.extractor_eval.fixture_dir);
    let dir = if dir.is_absolute() {
        dir
    } else {
        std::env::current_dir()?.join(dir)
    };
    archon_reasoning_quality::fixtures::load_labeled_turns(&dir)
}

fn open_store(root: &Path) -> Result<archon_reasoning_quality::store::ReasoningQualityStore> {
    archon_reasoning_quality::store::ReasoningQualityStore::open(root)
}

fn reasoning_quality_root() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?
        .join(".archon")
        .join("reasoning-quality"))
}

fn world_model_root() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?
        .join(".archon")
        .join("world-model"))
}

fn open_learning_db() -> Result<cozo::DbInstance> {
    let db =
        crate::command::store_paths::open_evidence_db("learning", &["ARCHON_LEARNING_DB_PATH"])?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

fn dead_letter_count(root: &Path) -> usize {
    let path = root.join("dead-letter").join("bridge-failures.jsonl");
    std::fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .unwrap_or(0)
}

#[derive(serde::Deserialize)]
struct DeadLetterEntry {
    bridge: String,
    #[serde(default)]
    event_json: Option<archon_reasoning_quality::ReasoningQualityEvent>,
}

fn read_dead_letters(root: &Path) -> Result<Vec<DeadLetterEntry>> {
    let path = root.join("dead-letter").join("bridge-failures.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for line in std::fs::read_to_string(path)?.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<DeadLetterEntry>(line) {
            entries.push(entry);
        }
    }
    Ok(entries)
}
