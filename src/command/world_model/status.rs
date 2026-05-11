use anyhow::Result;

pub(crate) fn render_world_status(config: &archon_core::config::ArchonConfig) -> String {
    let stats = load_world_model_stats().unwrap_or_default();
    render_world_status_with_stats(config, stats)
}

pub(crate) fn load_world_model_stats() -> Result<archon_world_model::ColdStartStats> {
    super::open_world_model_store()?.cold_start_stats()
}

pub(crate) fn render_world_status_with_stats(
    config: &archon_core::config::ArchonConfig,
    stats: archon_world_model::ColdStartStats,
) -> String {
    let wm = &config.learning.world_model;
    let cold = archon_world_model::trace::evaluate_cold_start(
        stats,
        archon_world_model::ColdStartThresholds {
            min_rows: wm.cold_start.min_rows,
            min_sessions: wm.cold_start.min_sessions,
            min_observed_days: wm.cold_start.min_observed_days,
        },
    );
    let cold_status = match cold {
        archon_world_model::ColdStartStatus::Ready => "ready".to_string(),
        archon_world_model::ColdStartStatus::ColdStart {
            rows_needed,
            sessions_needed,
            days_needed,
        } => format!(
            "cold_start (needs {rows_needed} rows, {sessions_needed} sessions, {days_needed} observed days)"
        ),
    };
    let active = super::active_model_id()
        .ok()
        .flatten()
        .unwrap_or_else(|| "none".into());
    let candidate_count = super::model_registry()
        .and_then(|registry| registry.candidate_count())
        .unwrap_or_default();
    let last_eval = last_eval_summary();
    let backend = archon_world_model::backend::select_runtime_backend(
        match wm.training.backend.as_str() {
            "cpu" => archon_world_model::backend::BackendKind::Cpu,
            "cuda" => archon_world_model::backend::BackendKind::Cuda,
            "metal" => archon_world_model::backend::BackendKind::Metal,
            _ => archon_world_model::backend::BackendKind::Auto,
        },
        wm.training.allow_cpu_fallback,
    );
    let advisor_status =
        if matches!(cold, archon_world_model::ColdStartStatus::Ready) && active != "none" {
            "ready"
        } else {
            "fail-open"
        };

    format!(
        "World Model Status\n\
         ==================\n\
         Enabled:            {enabled}\n\
         Model kind:         {model_kind}\n\
         State dim:          {state_dim}\n\
         Training backend:   {backend}\n\
         Precision:          {precision}\n\
         Eval parity:        {parity_precision} cosine >= {parity_min_cosine}\n\
         Last eval:          {last_eval}\n\
         Corpus rows:        {rows}\n\
         Corpus sessions:    {sessions}\n\
         Observed days:      {observed_days}\n\
         Cold-start status:  {cold_status}\n\
         Active model:       {active}\n\
         Candidate models:   {candidate_count}\n\
         Selected backend:   {selected_backend}\n\
         Backend fallback:   {fallback}\n\
         Auto-trainer:       {auto_trainer}\n\
         Trainer status:     idle-aware\n\
         Advisor status:     {advisor_status}\n\
         Advisor mode:       advisory",
        enabled = wm.enabled,
        model_kind = wm.model_kind,
        state_dim = wm.state_dim,
        backend = wm.training.backend,
        precision = wm.training.precision,
        parity_precision = wm.eval.parity_precision,
        parity_min_cosine = wm.eval.parity_min_cosine,
        rows = stats.rows,
        sessions = stats.sessions,
        observed_days = stats.observed_days,
        selected_backend = backend.selected,
        fallback = backend.fallback_reason.as_deref().unwrap_or("none"),
        auto_trainer = wm.auto_trainer.enabled,
    )
}

fn last_eval_summary() -> String {
    let Ok(Some(eval)) = super::model_registry().and_then(|registry| registry.latest_eval_report())
    else {
        return "none".into();
    };
    format!(
        "{} gates={} cosine={} surprise={} counterfactual={} brier={}",
        eval.candidate_id,
        eval.report.all_primary_gates_passed(),
        eval.report.cosine_error_improved,
        eval.report.surprise_ks_passed,
        eval.report.counterfactual_ndcg_passed,
        eval.report.brier_improved
    )
}
