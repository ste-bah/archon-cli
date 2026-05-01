//! WasmPluginHost — wasmtime Engine + Store setup for TASK-CLI-301.
//!
//! TASK-CLI-500: Added `WasmRuntime` to hold a live Store+Instance after loading,
//! and `dispatch_tool()` to call into the WASM guest for tool invocations.

use std::path::{Path, PathBuf};

use wasmtime::{Caller, Config, Engine, Extern, Instance, Linker, Module, Store};

use crate::abi::{
    DEFAULT_FUEL_BUDGET, DEFAULT_MAX_MEMORY_BYTES, HOST_API_VERSION, MIN_SUPPORTED_GUEST_VERSION,
    VALID_HOOK_EVENTS, read_guest_str, write_guest_i32, write_guest_memory,
};
use crate::capability::PluginCapability;
use crate::error::PluginError;
use crate::instance::{PluginInstance, RegisteredTool};

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

// ── WASM tool dispatch ────────────────────────────────────────────────────────

/// Call `archon_call_tool` in the WASM guest.
///
/// ABI:
/// ```text
/// archon_call_tool(
///   name_ptr: i32, name_len: i32,   // tool name in guest memory
///   args_ptr: i32, args_len: i32,   // args JSON in guest memory
///   result_ptr: i32, result_len_ptr: i32,  // result buffer + ptr to write length
/// ) -> i32  // 0 = ok, negative = error
/// ```
///
/// The caller allocates all buffers via the guest `alloc` export.
fn call_wasm_tool(
    runtime: &mut WasmRuntime,
    tool_name: &str,
    args_json: &str,
) -> anyhow::Result<String> {
    const RESULT_MAX_BYTES: i32 = 65536;

    // Get required functions — use `&mut *store` at each site to force a reborrow
    // instead of a move, allowing multiple uses of the same store reference.
    let alloc = runtime
        .instance
        .get_typed_func::<i32, i32>(&mut runtime.store, "alloc")
        .map_err(|e| anyhow::anyhow!("missing alloc: {e}"))?;
    let call_tool = runtime
        .instance
        .get_typed_func::<(i32, i32, i32, i32, i32, i32), i32>(
            &mut runtime.store,
            "archon_call_tool",
        )
        .map_err(|e| anyhow::anyhow!("missing archon_call_tool: {e}"))?;
    let memory = runtime
        .instance
        .get_memory(&mut runtime.store, "memory")
        .ok_or_else(|| anyhow::anyhow!("missing memory export"))?;

    let name_bytes = tool_name.as_bytes();
    let args_bytes = args_json.as_bytes();

    // Allocate guest memory for name
    let name_len = name_bytes.len() as i32;
    let name_ptr = alloc.call(&mut runtime.store, name_len)?;

    // Write name into guest memory
    {
        let mem = memory.data_mut(&mut runtime.store);
        let start = name_ptr as usize;
        let end = start + name_bytes.len();
        if end > mem.len() {
            anyhow::bail!("guest memory too small for tool name");
        }
        mem[start..end].copy_from_slice(name_bytes);
    }

    // Allocate guest memory for args
    let args_len = args_bytes.len().max(1) as i32; // at least 1 to avoid zero-alloc
    let args_ptr = alloc.call(&mut runtime.store, args_len)?;

    // Write args into guest memory
    {
        let mem = memory.data_mut(&mut runtime.store);
        let start = args_ptr as usize;
        let end = start + args_bytes.len();
        if end > mem.len() {
            anyhow::bail!("guest memory too small for args");
        }
        if !args_bytes.is_empty() {
            mem[start..end].copy_from_slice(args_bytes);
        }
    }

    // Allocate result buffer
    let result_ptr = alloc.call(&mut runtime.store, RESULT_MAX_BYTES)?;
    // Allocate 4-byte slot for the result length (written by guest as LE i32)
    let result_len_ptr = alloc.call(&mut runtime.store, 4)?;

    // Call archon_call_tool
    let ret = call_tool.call(
        &mut runtime.store,
        (
            name_ptr,
            name_len,
            args_ptr,
            args_bytes.len() as i32,
            result_ptr,
            result_len_ptr,
        ),
    )?;

    if ret < 0 {
        anyhow::bail!("archon_call_tool returned error code {ret}");
    }

    // Read result length from guest memory
    let result_bytes_written = {
        let mem = memory.data(&runtime.store);
        let start = result_len_ptr as usize;
        if start + 4 > mem.len() {
            anyhow::bail!("result_len_ptr out of bounds");
        }
        let len_bytes: [u8; 4] = mem[start..start + 4].try_into().unwrap();
        i32::from_le_bytes(len_bytes) as usize
    };

    // Read result bytes from guest memory
    let result = {
        let mem = memory.data(&runtime.store);
        let start = result_ptr as usize;
        let end = start + result_bytes_written.min(RESULT_MAX_BYTES as usize);
        if end > mem.len() {
            anyhow::bail!("result_ptr range out of bounds");
        }
        String::from_utf8_lossy(&mem[start..end]).into_owned()
    };

    // Validate it's JSON; if not, wrap it.
    if serde_json::from_str::<serde_json::Value>(&result).is_ok() {
        Ok(result)
    } else {
        Ok(serde_json::json!({"result": result}).to_string())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn verify_required_exports(
    instance: &Instance,
    store: &mut Store<PluginData>,
    path: &Path,
) -> Result<(), PluginError> {
    let export_names: Vec<String> = instance
        .exports(store)
        .map(|e| e.name().to_string())
        .collect();

    for required in &["alloc", "dealloc", "archon_guest_api_version", "memory"] {
        if !export_names.iter().any(|n| n == required) {
            return Err(PluginError::ComponentLoadFailed {
                path: path.to_path_buf(),
                reason: format!("missing export '{required}'"),
            });
        }
    }
    Ok(())
}

fn call_guest_api_version(
    instance: &Instance,
    store: &mut Store<PluginData>,
) -> Result<u32, PluginError> {
    let func = instance
        .get_typed_func::<(), i32>(&mut *store, "archon_guest_api_version")
        .map_err(|_| PluginError::AbiMismatch {
            expected: HOST_API_VERSION,
            got: 0,
        })?;
    let version = func
        .call(store, ())
        .map_err(|e| PluginError::LoadFailed(format!("archon_guest_api_version: {e}")))?;
    Ok(version as u32)
}

fn get_memory(caller: &mut Caller<PluginData>) -> Option<wasmtime::Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(mem)) => Some(mem),
        _ => None,
    }
}

// ── Host function registration ────────────────────────────────────────────────

fn register_host_functions(linker: &mut Linker<PluginData>) -> anyhow::Result<()> {
    // archon_log(level, msg_ptr, msg_len)
    linker.func_wrap(
        "archon",
        "archon_log",
        |mut caller: Caller<PluginData>, level: i32, ptr: i32, len: i32| {
            if let Some(mem) = get_memory(&mut caller) {
                let data = mem.data(&caller);
                if let Some(msg) = read_guest_str(data, ptr, len) {
                    let plugin = caller.data().plugin_name.clone();
                    match level {
                        0 => tracing::trace!("[plugin:{plugin}] {msg}"),
                        1 => tracing::debug!("[plugin:{plugin}] {msg}"),
                        2 => tracing::info!("[plugin:{plugin}] {msg}"),
                        3 => tracing::warn!("[plugin:{plugin}] {msg}"),
                        _ => tracing::error!("[plugin:{plugin}] {msg}"),
                    }
                }
            }
        },
    )?;

    // archon_register_tool(name_ptr, name_len, schema_ptr, schema_len)
    linker.func_wrap(
        "archon",
        "archon_register_tool",
        |mut caller: Caller<PluginData>, np: i32, nl: i32, sp: i32, sl: i32| {
            let has_cap = caller
                .data()
                .capabilities
                .iter()
                .any(|c| matches!(c, PluginCapability::ToolRegister));
            if !has_cap {
                return;
            }
            if let Some(mem) = get_memory(&mut caller) {
                let (name, schema) = {
                    let data = mem.data(&caller);
                    (
                        read_guest_str(data, np, nl).unwrap_or_default(),
                        read_guest_str(data, sp, sl).unwrap_or_default(),
                    )
                };
                caller.data_mut().registered_tools.push(RegisteredTool {
                    name,
                    schema_json: schema,
                });
            }
        },
    )?;

    // archon_register_hook(event_ptr, event_len) -> i32
    linker.func_wrap(
        "archon",
        "archon_register_hook",
        |mut caller: Caller<PluginData>, ep: i32, el: i32| -> i32 {
            let has_cap = caller
                .data()
                .capabilities
                .iter()
                .any(|c| matches!(c, PluginCapability::HookRegister));
            if !has_cap {
                return 2;
            }
            if let Some(mem) = get_memory(&mut caller) {
                let event = {
                    let data = mem.data(&caller);
                    read_guest_str(data, ep, el).unwrap_or_default()
                };
                if VALID_HOOK_EVENTS.contains(&event.as_str()) {
                    caller.data_mut().registered_hooks.push(event);
                    return 0;
                }
            }
            1
        },
    )?;

    // archon_register_command(name_ptr, name_len)
    linker.func_wrap(
        "archon",
        "archon_register_command",
        |mut caller: Caller<PluginData>, np: i32, nl: i32| {
            let has_cap = caller
                .data()
                .capabilities
                .iter()
                .any(|c| matches!(c, PluginCapability::CommandRegister));
            if !has_cap {
                return;
            }
            if let Some(mem) = get_memory(&mut caller) {
                let name = {
                    let data = mem.data(&caller);
                    read_guest_str(data, np, nl).unwrap_or_default()
                };
                caller.data_mut().registered_commands.push(name);
            }
        },
    )?;

    // archon_host_call(fp, fl, ap, al, rp, rl) -> i32
    linker.func_wrap(
        "archon",
        "archon_host_call",
        |mut caller: Caller<PluginData>,
         fp: i32,
         fl: i32,
         ap: i32,
         al: i32,
         rp: i32,
         rl: i32|
         -> i32 {
            if let Some(mem) = get_memory(&mut caller) {
                let (func_name, args_str) = {
                    let data = mem.data(&caller);
                    (
                        read_guest_str(data, fp, fl).unwrap_or_default(),
                        read_guest_str(data, ap, al).unwrap_or_default(),
                    )
                };
                let plugin = caller.data().plugin_name.clone();
                let result = dispatch_host_call(&func_name, &args_str, &plugin);
                let result_bytes = result.into_bytes();
                let mem_data = mem.data_mut(&mut caller);
                if write_guest_memory(mem_data, rp, &result_bytes) {
                    write_guest_i32(mem_data, rl, result_bytes.len() as i32);
                    0
                } else {
                    -1
                }
            } else {
                -2
            }
        },
    )?;

    // archon_api_version() -> i32
    linker.func_wrap(
        "archon",
        "archon_api_version",
        |_: Caller<PluginData>| -> i32 { HOST_API_VERSION as i32 },
    )?;

    Ok(())
}

fn dispatch_host_call(func: &str, _args: &str, plugin: &str) -> String {
    match func {
        "ping" => "pong".to_string(),
        "version" => HOST_API_VERSION.to_string(),
        other => {
            tracing::debug!("[plugin:{plugin}] archon_host_call: unknown '{other}'");
            format!("{{\"error\":\"unknown function: {other}\"}}")
        }
    }
}
