//! PluginLoader — discover, validate, and load plugins from directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::cache::WasmCache;
use crate::capability::PluginCapability;
use crate::error::PluginError;
use crate::manifest::load_manifest;
use crate::result::{LoadedPlugin, PluginLoadResult, PluginLoadWarning};
use crate::types::{ManifestCapability, StructuredCapability};

// ── PluginLoader ──────────────────────────────────────────────────────────────

/// Discovers and loads plugins from a plugin directory.
///
/// ## Loading semantics
/// - Plugins are directories inside `plugins_dir` (and optionally `seed_dirs`).
/// - Each must have `.archon-plugin/plugin.json`.
/// - Missing manifests are silently skipped (not errors).
/// - Parse/validation failures produce typed errors in `PluginLoadResult.errors`.
/// - Capability mismatches produce `PluginError::CapabilityDenied` errors.
/// - Dependency failures produce `PluginError::DependencyUnsatisfied` errors.
/// - Loading is fail-open: errors never block startup.
pub struct PluginLoader {
    plugins_dir: PathBuf,
    seed_dirs: Vec<PathBuf>,
    granted_capabilities: Vec<PluginCapability>,
    enabled_plugins: HashMap<String, bool>,
    cache: Option<WasmCache>,
}

impl PluginLoader {
    /// Create a loader for `plugins_dir`.
    ///
    /// The directory is created on `load_all()` if it does not exist.
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self {
            plugins_dir,
            seed_dirs: Vec::new(),
            granted_capabilities: Vec::new(),
            enabled_plugins: HashMap::new(),
            cache: None,
        }
    }

    /// Add seed directories (read-only fallback layers).
    pub fn with_seed_dirs(mut self, dirs: Vec<PathBuf>) -> Self {
        self.seed_dirs = dirs;
        self
    }

    /// Set the capabilities granted by the user in config.
    pub fn with_granted_capabilities(mut self, caps: Vec<PluginCapability>) -> Self {
        self.granted_capabilities = caps;
        self
    }

    /// Set per-plugin enable/disable state. Missing key = enabled.
    pub fn with_enabled_state(mut self, state: HashMap<String, bool>) -> Self {
        self.enabled_plugins = state;
        self
    }

    /// Attach a WASM compilation cache.
    pub fn with_cache(mut self, cache: WasmCache) -> Self {
        self.cache = Some(cache);
        self
    }

    // ── Entry point ───────────────────────────────────────────────────────

    /// Scan all plugin directories and return a `PluginLoadResult`.
    ///
    /// Creates `plugins_dir` if it does not exist. Loading is fail-open.
    pub fn load_all(&self) -> PluginLoadResult {
        // Ensure main plugins dir exists
        let _ = std::fs::create_dir_all(&self.plugins_dir);

        // Collect candidate (name → dir) pairs, main dir takes precedence over seeds
        let candidates = self.collect_candidates();

        let mut result = PluginLoadResult::default();
        let mut loaded_names: Vec<String> = Vec::new();

        // Two-pass: first pass collects all valid manifests; second pass checks deps.
        // We do single-pass with dependency check against already-loaded set.
        //
        // Stable order: sort by name for determinism.
        let mut sorted: Vec<(String, PathBuf)> = candidates.into_iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));

        // We need two passes: first load independent plugins, then dependents.
        // Simple approach: keep iterating until no progress.
        let mut pending: Vec<(String, PathBuf)> = sorted;
        let mut made_progress = true;
        while made_progress && !pending.is_empty() {
            made_progress = false;
            let mut next_pending = Vec::new();
            for (dir_name, plugin_dir) in pending {
                let outcome = self.load_one(&dir_name, &plugin_dir, &loaded_names);
                match outcome {
                    LoadOutcome::Enabled(lp, warnings) => {
                        loaded_names.push(lp.plugin_id.clone());
                        result
                            .warnings
                            .extend(warnings.into_iter().map(|w| (lp.plugin_id.clone(), w)));
                        result.enabled.push(lp);
                        made_progress = true;
                    }
                    LoadOutcome::Disabled(lp, warnings) => {
                        loaded_names.push(lp.plugin_id.clone());
                        result
                            .warnings
                            .extend(warnings.into_iter().map(|w| (lp.plugin_id.clone(), w)));
                        result.disabled.push(lp);
                        made_progress = true;
                    }
                    LoadOutcome::Error(id, err) => {
                        result.errors.push((id, err));
                        made_progress = true;
                    }
                    LoadOutcome::DepPending => {
                        // Retry after others have loaded
                        next_pending.push((dir_name, plugin_dir));
                    }
                    LoadOutcome::Skip => {
                        made_progress = true;
                    }
                }
            }
            pending = next_pending;
        }

        // Anything still pending after no more progress = unsatisfied dependency
        for (dir_name, plugin_dir) in pending {
            let id = infer_plugin_id(&dir_name, &plugin_dir);
            result.errors.push((
                id.clone(),
                PluginError::DependencyUnsatisfied {
                    plugin: id,
                    dependency: "dependency cycle or missing dependency".to_string(),
                },
            ));
        }

        result
    }

    // ── Internals ─────────────────────────────────────────────────────────

    /// Collect all candidate plugin directories: main dir + seed dirs.
    /// Main dir takes precedence (seed dirs' entries are ignored if same name exists in main).
    fn collect_candidates(&self) -> HashMap<String, PathBuf> {
        let mut candidates: HashMap<String, PathBuf> = HashMap::new();

        // Seeds first (lowest precedence)
        for seed in &self.seed_dirs {
            self.scan_dir(seed, &mut candidates, false);
        }

        // Main dir (highest precedence — overwrites seed entries)
        self.scan_dir(&self.plugins_dir, &mut candidates, true);

        candidates
    }

    fn scan_dir(
        &self,
        dir: &std::path::Path,
        candidates: &mut HashMap<String, PathBuf>,
        overwrite: bool,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if overwrite || !candidates.contains_key(&name) {
                    candidates.insert(name, path);
                }
            }
        }
    }

    /// Attempt to load one plugin. Returns the outcome.
    fn load_one(&self, dir_name: &str, plugin_dir: &Path, loaded_names: &[String]) -> LoadOutcome {
        // 1. Load manifest (None = no manifest file = silent skip)
        let manifest = match load_manifest(plugin_dir) {
            None => return LoadOutcome::Skip,
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                let id = dir_name.to_string();
                return LoadOutcome::Error(id, e);
            }
        };

        let plugin_id = manifest.name.clone();

        // 2. Check capabilities
        let mut warnings = Vec::new();
        for manifest_capability in &manifest.capabilities {
            let requested = capability_from_manifest(manifest_capability);
            if !self.is_capability_granted(&requested, &mut warnings) {
                return LoadOutcome::Error(plugin_id, PluginError::CapabilityDenied(requested));
            }
        }

        // Check dependencies — if any are not yet loaded, defer until next iteration.
        for dep in &manifest.dependencies {
            let dep_name = dep.split('@').next().unwrap_or(dep.as_str());
            if !loaded_names.iter().any(|n| n == dep_name) {
                return LoadOutcome::DepPending;
            }
        }

        // 5. Create data directory
        let data_dir = self.plugins_dir.join("data").join(&plugin_id);
        if std::fs::create_dir_all(&data_dir).is_err() {
            // Non-fatal — log and continue
            tracing::warn!("failed to create data dir for plugin '{plugin_id}'");
        }

        // 6. Check wasm path (optional)
        let wasm_path = {
            let p = plugin_dir.join("plugin.wasm");
            if p.exists() { Some(p) } else { None }
        };

        let lp = LoadedPlugin {
            plugin_id: plugin_id.clone(),
            manifest,
            data_dir,
            wasm_path,
        };

        // 7. Enable/disable check
        let enabled = *self.enabled_plugins.get(&plugin_id).unwrap_or(&true);
        if enabled {
            LoadOutcome::Enabled(lp, warnings)
        } else {
            LoadOutcome::Disabled(lp, warnings)
        }
    }

    fn is_capability_granted(
        &self,
        requested: &PluginCapability,
        warnings: &mut Vec<PluginLoadWarning>,
    ) -> bool {
        for granted in &self.granted_capabilities {
            if let Some(warning) = capability_matches(granted, requested) {
                if let GrantWarning::WildcardNetwork { approval } = warning {
                    let requested_hosts = match requested {
                        PluginCapability::Network(hosts) => hosts.clone(),
                        _ => Vec::new(),
                    };
                    tracing::warn!(
                        approval = %approval,
                        requested_hosts = ?requested_hosts,
                        "plugin network capability granted by wildcard operator approval"
                    );
                    warnings.push(PluginLoadWarning::WildcardNetworkGrant {
                        requested_hosts,
                        approval,
                    });
                }
                return true;
            }
        }
        false
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

enum LoadOutcome {
    Enabled(LoadedPlugin, Vec<PluginLoadWarning>),
    Disabled(LoadedPlugin, Vec<PluginLoadWarning>),
    Error(String, PluginError),
    /// Dependencies not yet loaded — retry next iteration.
    DepPending,
    /// No manifest file — silent skip.
    Skip,
}

/// Infer plugin ID from directory name (fallback when manifest can't be parsed).
fn infer_plugin_id(dir_name: &str, _plugin_dir: &PathBuf) -> String {
    dir_name.to_string()
}

fn capability_from_manifest(capability: &ManifestCapability) -> PluginCapability {
    match capability {
        ManifestCapability::Legacy(_) => PluginCapability::None,
        ManifestCapability::Structured(cap) => capability_from_structured(cap),
    }
}

fn capability_from_structured(cap: &StructuredCapability) -> PluginCapability {
    match cap.kind.as_str() {
        "ReadFs" => PluginCapability::ReadFs(cap.paths.clone()),
        "WriteFs" => PluginCapability::WriteFs(cap.paths.clone()),
        "Network" => PluginCapability::Network(cap.hosts.clone()),
        "ToolRegister" => PluginCapability::ToolRegister,
        "HookRegister" => PluginCapability::HookRegister,
        "CommandRegister" => PluginCapability::CommandRegister,
        "LspRegister" => PluginCapability::LspRegister,
        "DataDirWrite" => PluginCapability::DataDirWrite,
        _ => PluginCapability::None,
    }
}

enum GrantWarning {
    None,
    WildcardNetwork { approval: String },
}

/// Check if a granted capability covers the structured capability request.
fn capability_matches(
    granted: &PluginCapability,
    requested: &PluginCapability,
) -> Option<GrantWarning> {
    match (granted, requested) {
        (PluginCapability::ReadFs(granted_paths), PluginCapability::ReadFs(requested_paths)) => {
            paths_cover(granted_paths, requested_paths).then_some(GrantWarning::None)
        }
        (PluginCapability::WriteFs(granted_paths), PluginCapability::WriteFs(requested_paths)) => {
            paths_cover(granted_paths, requested_paths).then_some(GrantWarning::None)
        }
        (PluginCapability::Network(granted_hosts), PluginCapability::Network(requested_hosts)) => {
            hosts_cover(granted_hosts, requested_hosts).then_some(GrantWarning::None)
        }
        (
            PluginCapability::NetworkWildcardApproved { approval },
            PluginCapability::Network(requested_hosts),
        ) if !requested_hosts.is_empty() => Some(GrantWarning::WildcardNetwork {
            approval: approval.clone(),
        }),
        (PluginCapability::ToolRegister, PluginCapability::ToolRegister)
        | (PluginCapability::HookRegister, PluginCapability::HookRegister)
        | (PluginCapability::CommandRegister, PluginCapability::CommandRegister)
        | (PluginCapability::LspRegister, PluginCapability::LspRegister)
        | (PluginCapability::DataDirWrite, PluginCapability::DataDirWrite) => {
            Some(GrantWarning::None)
        }
        _ => None,
    }
}

fn paths_cover(granted_paths: &[PathBuf], requested_paths: &[PathBuf]) -> bool {
    !requested_paths.is_empty()
        && requested_paths.iter().all(|requested| {
            granted_paths
                .iter()
                .any(|granted| requested.starts_with(granted))
        })
}

fn hosts_cover(granted_hosts: &[String], requested_hosts: &[String]) -> bool {
    !requested_hosts.is_empty()
        && requested_hosts
            .iter()
            .all(|requested| granted_hosts.iter().any(|granted| granted == requested))
}

// ── instantiate_wasm_plugins ──────────────────────────────────────────────────

/// Compile and instantiate all enabled WASM plugins from a load result.
///
/// Returns a map of `plugin_id → (PluginInstance, Arc<Mutex<WasmPluginHost>>)`.
/// Fail-open: any plugin whose WASM fails to load is logged and skipped.
/// Plugins without a `wasm_path` are silently skipped.
pub fn instantiate_wasm_plugins(
    result: &crate::result::PluginLoadResult,
) -> HashMap<
    String,
    (
        crate::instance::PluginInstance,
        std::sync::Arc<std::sync::Mutex<crate::host::WasmPluginHost>>,
    ),
> {
    use crate::host::{PluginHostConfig, WasmPluginHost};
    use std::sync::{Arc, Mutex};

    let mut out = HashMap::new();

    for plugin in &result.enabled {
        let Some(ref wasm_path) = plugin.wasm_path else {
            continue;
        };

        let wasm_bytes = match std::fs::read(wasm_path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(plugin = %plugin.plugin_id, "failed to read WASM bytes: {e}");
                continue;
            }
        };

        let caps: Vec<PluginCapability> = plugin
            .manifest
            .capabilities
            .iter()
            .map(capability_from_manifest)
            .collect();

        let mut host = match WasmPluginHost::new(PluginHostConfig::default()) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(plugin = %plugin.plugin_id, "failed to create WASM host: {e}");
                continue;
            }
        };

        match host.load_plugin(
            wasm_bytes,
            caps,
            Some(&plugin.plugin_id),
            plugin.data_dir.clone(),
        ) {
            Ok(instance) => {
                let id = plugin.plugin_id.clone();
                tracing::info!(plugin = %id, "WASM plugin instantiated");
                out.insert(id, (instance, Arc::new(Mutex::new(host))));
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %plugin.plugin_id,
                    "WASM instantiation failed (fail-open): {e}"
                );
            }
        }
    }

    out
}
