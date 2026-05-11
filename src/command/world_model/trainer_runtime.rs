pub(crate) fn schedule_dynamic_trainer_tick(config: archon_core::config::ArchonConfig) {
    if !config.learning.world_model.enabled || !config.learning.world_model.auto_trainer.enabled {
        return;
    }
    archon_observability::spawn_named("world-model-dynamic-trainer", async move {
        let Ok(root) = super::world_model_root() else {
            return;
        };
        let auto = &config.learning.world_model.auto_trainer;
        let _ = super::candidate::render_trainer_tick(
            &config,
            &root,
            Some(auto.idle_required_ms),
            None,
            None,
            false,
        );
    });
}
