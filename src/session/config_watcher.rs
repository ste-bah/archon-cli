use std::path::PathBuf;
use std::sync::Arc;

use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::observability;

pub(super) fn spawn_config_watcher(
    config_path: PathBuf,
    config: archon_core::config::ArchonConfig,
    tui_tx: TuiEventSender,
    hook_registry: Arc<archon_core::hooks::HookRegistry>,
    working_dir: PathBuf,
    session_id: String,
) {
    let config_paths = vec![config_path];
    match archon_core::config_watcher::ConfigWatcher::start(&config_paths) {
        Ok(watcher) => {
            let reloader =
                archon_core::config_watcher::DebouncedReloader::new(watcher, 500, config);
            observability::spawn_named("config-watcher", async move {
                let mut reloader = reloader;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if let Some(changed_keys) = reloader.check_and_reload(&config_paths) {
                        if !changed_keys.is_empty() {
                            hook_registry
                                .execute_hooks(
                                    archon_core::hooks::HookEvent::ConfigChange,
                                    serde_json::json!({
                                        "hook_event": "ConfigChange",
                                        "changed_keys": changed_keys,
                                    }),
                                    &working_dir,
                                    &session_id,
                                )
                                .await;
                        }

                        let non_reloadable =
                            archon_core::config_diff::non_reloadable_changes(&changed_keys);
                        if !non_reloadable.is_empty() {
                            let msg = format!(
                                "\nConfig reloaded. Non-reloadable changes (require restart): {}\n",
                                non_reloadable.join(", ")
                            );
                            let _ = tui_tx.send(TuiEvent::TextDelta(msg));
                        } else if !changed_keys.is_empty() {
                            let msg = format!("\nConfig reloaded: {}\n", changed_keys.join(", "));
                            let _ = tui_tx.send(TuiEvent::TextDelta(msg));
                        }
                    }
                }
            });
            tracing::debug!("config file watcher started");
        }
        Err(e) => tracing::warn!("failed to start config watcher: {e}"),
    }
}
