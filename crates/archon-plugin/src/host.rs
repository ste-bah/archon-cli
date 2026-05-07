//! WasmPluginHost — wasmtime Engine + Store setup for TASK-CLI-301.
//!
//! TASK-CLI-500: Added `WasmRuntime` to hold a live Store+Instance after loading,
//! and `dispatch_tool()` to call into the WASM guest for tool invocations.

use std::path::PathBuf;

use wasmtime::{Config, Engine, Instance, Linker, Module, Store};

use crate::abi::{
    DEFAULT_FUEL_BUDGET, DEFAULT_MAX_MEMORY_BYTES, HOST_API_VERSION, MIN_SUPPORTED_GUEST_VERSION,
};
use crate::capability::PluginCapability;
use crate::error::PluginError;
use crate::instance::{PluginInstance, RegisteredTool};

mod dispatch;
mod host_functions;

use dispatch::call_wasm_tool;
use host_functions::{call_guest_api_version, register_host_functions, verify_required_exports};

// ── PluginHostConfig ──────────────────────────────────────────────────────────

/// Configuration for the WASM plugin host.
#[derive(Debug, Clone)]
pub struct PluginHostConfig {
    /// Maximum linear memory per plugin instance (bytes). Default: 64 MiB.
    pub max_memory_bytes: usize,
    /// Fuel budget per store (consumed across all WASM instructions). Default: 10M.
    pub fuel_budget: u64,
}

impl Default for PluginHostConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            fuel_budget: DEFAULT_FUEL_BUDGET,
        }
    }
}

// ── Memory limiter ────────────────────────────────────────────────────────────

struct MemoryLimiter {
    max_bytes: usize,
}

impl wasmtime::ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(desired <= self.max_bytes)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }
}

// ── Store user data ───────────────────────────────────────────────────────────

struct PluginData {
    capabilities: Vec<PluginCapability>,
    plugin_name: String,
    registered_tools: Vec<RegisteredTool>,
    registered_hooks: Vec<String>,
    registered_commands: Vec<String>,
    limiter: MemoryLimiter,
}

// ── WasmRuntime ───────────────────────────────────────────────────────────────

/// A live wasmtime Store+Instance kept alive after loading for subsequent
/// `dispatch_tool()` calls.
///
/// Held exclusively inside `WasmPluginHost`. Wrapped in `Arc<Mutex<WasmPluginHost>>`
/// by callers that need thread-safe dispatch.
struct WasmRuntime {
    store: Store<PluginData>,
    instance: Instance,
    /// True if the module exports `archon_call_tool`. Set at load time.
    has_call_tool_export: bool,
}

// ── WasmPluginHost ────────────────────────────────────────────────────────────

/// The WASM plugin host: manages the wasmtime Engine and a single loaded plugin.
///
/// `load_plugin()` compiles, links, and instantiates the module, storing the live
/// runtime for subsequent `dispatch_tool()` calls.
///
/// Design: one host per plugin. Thread-safe dispatch requires wrapping in
/// `Arc<std::sync::Mutex<WasmPluginHost>>`.
pub struct WasmPluginHost {
    engine: Engine,
    config: PluginHostConfig,
    /// Set after a successful `load_plugin()`. `None` means not yet loaded or load failed.
    runtime: Option<WasmRuntime>,
    /// Plugin name, stored for tracing. Set during load_plugin().
    plugin_name_for_log: String,
}

impl WasmPluginHost {
    /// Initialize the host with the given configuration.
    ///
    /// Creates a wasmtime [`Engine`] with fuel metering enabled.
    pub fn new(config: PluginHostConfig) -> Result<Self, PluginError> {
        let mut wasm_config = Config::new();
        wasm_config.consume_fuel(true);
        let engine = Engine::new(&wasm_config)
            .map_err(|e| PluginError::LoadFailed(format!("engine init: {e}")))?;
        Ok(Self {
            engine,
            config,
            runtime: None,
            plugin_name_for_log: String::new(),
        })
    }

    /// Load a plugin from raw WASM bytes, retaining the live runtime for dispatch.
    ///
    /// On success, the `Store<PluginData>` and `Instance` are stored in `self.runtime`
    /// so `dispatch_tool()` can call back into the WASM module later.
    ///
    /// On failure, `self.runtime` remains `None` — `dispatch_tool()` returns an error
    /// JSON without panicking.
    pub fn load_plugin(
        &mut self,
        wasm_bytes: Vec<u8>,
        capabilities: Vec<PluginCapability>,
        plugin_name: Option<&str>,
        data_dir: PathBuf,
    ) -> Result<PluginInstance, PluginError> {
        let name = plugin_name.unwrap_or("unknown").to_string();
        self.plugin_name_for_log = name.clone();

        // 1. Compile module
        let module = Module::from_binary(&self.engine, &wasm_bytes)
            .map_err(|e| PluginError::LoadFailed(format!("compile: {e}")))?;

        // 2. Build store
        let data = PluginData {
            capabilities,
            plugin_name: name.clone(),
            registered_tools: Vec::new(),
            registered_hooks: Vec::new(),
            registered_commands: Vec::new(),
            limiter: MemoryLimiter {
                max_bytes: self.config.max_memory_bytes,
            },
        };
        let mut store = Store::new(&self.engine, data);
        store.limiter(|d| &mut d.limiter as &mut dyn wasmtime::ResourceLimiter);

        // 3. Set fuel
        store
            .set_fuel(self.config.fuel_budget)
            .map_err(|e| PluginError::LoadFailed(format!("set fuel: {e}")))?;

        // 4. Build linker
        let mut linker: Linker<PluginData> = Linker::new(&self.engine);
        register_host_functions(&mut linker).map_err(|e| PluginError::ComponentLoadFailed {
            path: data_dir.clone(),
            reason: format!("linker: {e}"),
        })?;

        // 5. Instantiate (runs start function if present — plugins register tools here)
        let instance = linker.instantiate(&mut store, &module).map_err(|e| {
            PluginError::ComponentLoadFailed {
                path: data_dir.clone(),
                reason: format!("instantiate: {e}"),
            }
        })?;

        // 6. Verify required exports exist
        verify_required_exports(&instance, &mut store, &data_dir)?;

        // 7. Negotiate API version
        let guest_version = call_guest_api_version(&instance, &mut store)?;
        if guest_version > HOST_API_VERSION || guest_version < MIN_SUPPORTED_GUEST_VERSION {
            return Err(PluginError::AbiMismatch {
                expected: HOST_API_VERSION,
                got: guest_version,
            });
        }

        // 8. Create data directory
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| PluginError::LoadFailed(format!("data_dir: {e}")))?;

        // 9. Check whether archon_call_tool is exported (optional).
        let has_call_tool_export = instance
            .get_export(&mut store, "archon_call_tool")
            .is_some();

        // 10. Extract plugin instance metadata from store data.
        //     We need registered_tools/hooks/commands before the store is stored.
        //     Clone the data out, then store the runtime.
        let registered_tools = store.data().registered_tools.clone();
        let registered_hooks = store.data().registered_hooks.clone();
        let registered_commands = store.data().registered_commands.clone();

        // 11. Store live runtime for dispatch.
        self.runtime = Some(WasmRuntime {
            store,
            instance,
            has_call_tool_export,
        });

        Ok(PluginInstance {
            plugin_name: name,
            data_dir,
            registered_tools,
            registered_hooks,
            registered_commands,
        })
    }

    /// Dispatch a tool call into the loaded WASM instance.
    ///
    /// Always emits a `tracing::info!` log before dispatching (Gate 3 requirement).
    /// Returns a JSON string. On any error (no runtime, no export, WASM error), returns
    /// a well-formed `{"error":"..."}` JSON string — never panics.
    pub fn dispatch_tool(&mut self, tool_name: &str, args_json: &str) -> String {
        tracing::info!(
            plugin = %self.plugin_name_for_log,
            tool = %tool_name,
            "WasmPluginHost::dispatch_tool called"
        );

        let Some(runtime) = &mut self.runtime else {
            return serde_json::json!({"error": "plugin not loaded"}).to_string();
        };

        if !runtime.has_call_tool_export {
            tracing::warn!(
                plugin = %self.plugin_name_for_log,
                tool = %tool_name,
                "WASM module does not export archon_call_tool"
            );
            return serde_json::json!({
                "error": "plugin does not implement archon_call_tool",
                "tool": tool_name
            })
            .to_string();
        }

        match call_wasm_tool(runtime, tool_name, args_json) {
            Ok(result) => result,
            Err(e) => {
                tracing::error!(
                    plugin = %self.plugin_name_for_log,
                    tool = %tool_name,
                    "WASM dispatch error: {e}"
                );
                serde_json::json!({"error": format!("WASM dispatch error: {e}")}).to_string()
            }
        }
    }
}
