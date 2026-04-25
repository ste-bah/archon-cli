//! TASK-AGS-507: Pattern plugin loader — native (inventory) and WASM plugins.
//!
//! Provides [`PatternPluginLoader`] which discovers native plugins registered
//! via `inventory::submit!` and loads WASM-based pattern plugins through
//! `wasmtime`. Both kinds are inserted into a shared [`PatternRegistry`].

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{Pattern, PatternCtx, PatternError, PatternKind, PatternRegistry};

// ---------------------------------------------------------------------------
// NativePluginDescriptor + inventory collection
// ---------------------------------------------------------------------------

/// Descriptor for a natively-compiled pattern plugin.
///
/// Register one via `inventory::submit!` in any crate linked into the binary.
/// [`PatternPluginLoader::load_native`] iterates all submitted descriptors at
/// startup and inserts version-compatible ones into the registry.
pub struct NativePluginDescriptor {
    /// Unique name used as the registry key.
    pub name: &'static str,
    /// Pattern schema version the plugin was compiled against.
    pub pattern_version: u32,
    /// Factory that produces a fresh `Arc<dyn Pattern>`.
    pub factory: fn() -> Arc<dyn Pattern>,
}

inventory::collect!(NativePluginDescriptor);

// ---------------------------------------------------------------------------
// WasmPluginConfig
// ---------------------------------------------------------------------------

/// Configuration for loading a single WASM pattern plugin from disk.
pub struct WasmPluginConfig {
    /// Path to the `.wasm` or `.wat` file.
    pub path: PathBuf,
    /// Name used as the registry key.
    pub name: String,
    /// Pattern schema version declared by the plugin.
    pub pattern_version: u32,
    /// Maximum fuel (instruction budget) per `execute` call.
    pub fuel: u64,
    /// Upper bound on WASM linear memory pages (currently informational).
    pub memory_max_pages: u32,
}

// ---------------------------------------------------------------------------
// WasmPattern
// ---------------------------------------------------------------------------

/// A pattern backed by a compiled WASM module.
///
/// Each [`execute`](Pattern::execute) call creates a **fresh**
/// [`wasmtime::Store`] so that fuel metering resets and no mutable state
/// leaks between invocations.
pub struct WasmPattern {
    engine: wasmtime::Engine,
    module: wasmtime::Module,
    name: String,
    fuel: u64,
}

#[async_trait]
impl Pattern for WasmPattern {
    fn kind(&self) -> PatternKind {
        PatternKind::Custom(self.name.clone())
    }

    async fn execute(&self, input: Value, _ctx: PatternCtx) -> Result<Value, PatternError> {
        // Serialize input to JSON bytes.
        let input_bytes = serde_json::to_vec(&input)
            .map_err(|e| PatternError::Execution(format!("wasm: {e}")))?;

        // Fresh store per call — isolation + fuel reset.
        let mut store = wasmtime::Store::new(&self.engine, ());
        store
            .set_fuel(self.fuel)
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        // Instantiate module (no imports required for our ABI).
        let instance = wasmtime::Instance::new(&mut store, &self.module, &[])
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        // Obtain exports.
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| {
            PatternError::Execution("wasm: module does not export 'memory'".into())
        })?;

        let alloc_fn = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        let execute_fn = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "pattern_execute")
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        // Allocate space inside the WASM linear memory for the input.
        let input_len = input_bytes.len() as i32;
        let input_ptr = alloc_fn
            .call(&mut store, input_len)
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        // Write the input bytes into WASM memory.
        memory
            .write(&mut store, input_ptr as usize, &input_bytes)
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        // Call the guest's pattern_execute.
        let packed = execute_fn
            .call(&mut store, (input_ptr, input_len))
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        // Unpack (out_ptr << 32) | out_len.
        let out_ptr = (packed >> 32) as usize;
        let out_len = (packed & 0xFFFF_FFFF) as usize;

        // Read output bytes from WASM memory.
        let mem_data = memory.data(&store);
        if out_ptr + out_len > mem_data.len() {
            return Err(PatternError::Execution(
                "wasm: output range exceeds linear memory".into(),
            ));
        }
        let out_bytes = &mem_data[out_ptr..out_ptr + out_len];

        // Parse as JSON.
        serde_json::from_slice(out_bytes).map_err(|e| PatternError::Execution(format!("wasm: {e}")))
    }
}

// ---------------------------------------------------------------------------
// PatternPluginLoader
// ---------------------------------------------------------------------------

/// Discovers and loads pattern plugins (native + WASM) into a
/// [`PatternRegistry`].
pub struct PatternPluginLoader {
    registry: Arc<PatternRegistry>,
    current_version: u32,
}

impl PatternPluginLoader {
    /// Create a new loader targeting `registry` with the given schema version.
    pub fn new(registry: Arc<PatternRegistry>, current_version: u32) -> Self {
        Self {
            registry,
            current_version,
        }
    }

    /// Returns `true` if version `v` is compatible: accepts the current
    /// version and N-1 (if current > 0).
    fn is_version_supported(&self, v: u32) -> bool {
        v == self.current_version || (self.current_version > 0 && v == self.current_version - 1)
    }

    /// Iterate all native plugins registered via `inventory::submit!`,
    /// version-check each one, and insert compatible plugins into the
    /// registry.
    ///
    /// Returns the count of plugins that were actually loaded.
    /// Version-mismatched plugins are logged at `warn` and skipped.
    pub fn load_native(&self) -> Result<usize, PatternError> {
        let mut count = 0usize;
        for desc in inventory::iter::<NativePluginDescriptor> {
            if !self.is_version_supported(desc.pattern_version) {
                tracing::warn!(
                    plugin = desc.name,
                    plugin_version = desc.pattern_version,
                    current_version = self.current_version,
                    "skipping native plugin: version mismatch"
                );
                continue;
            }
            let pattern = (desc.factory)();
            self.registry.register(desc.name, pattern);
            count += 1;
        }
        Ok(count)
    }

    /// Load a WASM pattern plugin from the path specified in `cfg`.
    ///
    /// The module is compiled with fuel metering enabled and registered under
    /// `cfg.name`.
    pub fn load_wasm(&self, cfg: WasmPluginConfig) -> Result<(), PatternError> {
        if !self.is_version_supported(cfg.pattern_version) {
            return Err(PatternError::Execution(format!(
                "wasm plugin '{}' has pattern_version {} (current {})",
                cfg.name, cfg.pattern_version, self.current_version,
            )));
        }

        let bytes = std::fs::read(&cfg.path).map_err(|e| {
            PatternError::Execution(format!("wasm: failed to read {:?}: {e}", cfg.path))
        })?;

        let mut engine_cfg = wasmtime::Config::new();
        engine_cfg.consume_fuel(true);

        let engine = wasmtime::Engine::new(&engine_cfg)
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        let module = wasmtime::Module::new(&engine, &bytes)
            .map_err(|e| PatternError::Execution(format!("wasm: {e:#}")))?;

        let pattern = Arc::new(WasmPattern {
            engine,
            module,
            name: cfg.name.clone(),
            fuel: cfg.fuel,
        }) as Arc<dyn Pattern>;

        self.registry.register(&cfg.name, pattern);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Test helper: a trivial echo pattern for native-plugin tests --------

    struct EchoPattern;

    #[async_trait]
    impl Pattern for EchoPattern {
        fn kind(&self) -> PatternKind {
            PatternKind::Custom("echo".into())
        }

        async fn execute(&self, input: Value, _ctx: PatternCtx) -> Result<Value, PatternError> {
            Ok(input)
        }
    }

    inventory::submit! {
        NativePluginDescriptor {
            name: "test_echo",
            pattern_version: 1,
            factory: || Arc::new(EchoPattern) as Arc<dyn Pattern>,
        }
    }

    // -- Helpers for building a PatternCtx (not exercised but required) -----

    struct DummyTaskService;

    #[async_trait]
    impl super::super::TaskServiceHandle for DummyTaskService {
        async fn submit(&self, _agent: &str, _input: Value) -> Result<Value, PatternError> {
            Ok(Value::Null)
        }
    }

    fn dummy_ctx(registry: &Arc<PatternRegistry>) -> PatternCtx {
        PatternCtx {
            task_service: Arc::new(DummyTaskService),
            registry: Arc::clone(registry),
            trace_id: "test-trace".into(),
            deadline: None,
        }
    }

    // -----------------------------------------------------------------------
    // 1. Native plugin loads and registers
    // -----------------------------------------------------------------------

    #[test]
    fn test_native_plugin_loads_and_registers() {
        let registry = Arc::new(PatternRegistry::new());
        let loader = PatternPluginLoader::new(Arc::clone(&registry), 1);
        let count = loader.load_native().expect("load_native should succeed");
        assert!(count >= 1, "at least the test_echo plugin should load");
        assert!(
            registry.resolve("test_echo").is_some(),
            "test_echo should be registered"
        );
    }

    // -----------------------------------------------------------------------
    // 2. Native plugin version mismatch is skipped (with warn log)
    // -----------------------------------------------------------------------

    #[tracing_test::traced_test]
    #[test]
    fn test_native_plugin_version_mismatch_skipped() {
        let registry = Arc::new(PatternRegistry::new());
        // current_version=99 — test_echo has version=1, which is neither 99 nor 98.
        let loader = PatternPluginLoader::new(Arc::clone(&registry), 99);
        let _count = loader.load_native().expect("load_native should succeed");
        assert!(
            registry.resolve("test_echo").is_none(),
            "test_echo should NOT be registered with version mismatch"
        );
        assert!(logs_contain("skipping native plugin: version mismatch"));
    }

    // -----------------------------------------------------------------------
    // 3. Native plugin version N-1 is accepted
    // -----------------------------------------------------------------------

    #[test]
    fn test_native_plugin_version_n_minus_1_accepted() {
        let registry = Arc::new(PatternRegistry::new());
        // current_version=2 — test_echo has version=1 = 2-1, should load.
        let loader = PatternPluginLoader::new(Arc::clone(&registry), 2);
        let count = loader.load_native().expect("load_native should succeed");
        assert!(
            count >= 1,
            "test_echo (version 1) should load under N-1 rule"
        );
        assert!(
            registry.resolve("test_echo").is_some(),
            "test_echo should be registered"
        );
    }

    // -----------------------------------------------------------------------
    // 4. Plugin pattern is interchangeable with built-in (US-PAT-07)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_us_pat_07_plugin_interchangeable_with_builtin() {
        let registry = Arc::new(PatternRegistry::new());

        // Register a "builtin" pattern manually.
        struct BuiltinDouble;

        #[async_trait]
        impl Pattern for BuiltinDouble {
            fn kind(&self) -> PatternKind {
                PatternKind::Pipeline
            }
            async fn execute(&self, input: Value, _ctx: PatternCtx) -> Result<Value, PatternError> {
                Ok(serde_json::json!({ "builtin": true, "input": input }))
            }
        }

        registry.register("builtin_double", Arc::new(BuiltinDouble));

        // Load native plugins (includes test_echo).
        let loader = PatternPluginLoader::new(Arc::clone(&registry), 1);
        loader.load_native().expect("load_native should succeed");

        // Resolve both through the same registry interface.
        let builtin: Arc<dyn Pattern> = registry
            .resolve("builtin_double")
            .expect("builtin_double should be registered");
        let plugin: Arc<dyn Pattern> = registry
            .resolve("test_echo")
            .expect("test_echo should be registered");

        let ctx_b = dummy_ctx(&registry);
        let ctx_p = dummy_ctx(&registry);

        let input = serde_json::json!({"data": 42});

        // Both execute through the same Arc<dyn Pattern> interface.
        let r_builtin = builtin.execute(input.clone(), ctx_b).await.unwrap();
        let r_plugin = plugin.execute(input.clone(), ctx_p).await.unwrap();

        // Builtin wraps input; plugin echoes it.
        assert!(r_builtin.get("builtin").is_some());
        assert_eq!(r_plugin, input);
    }

    // -----------------------------------------------------------------------
    // 5. WASM plugin fuel limit is enforced
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_wasm_plugin_fuel_limit_enforced() {
        // A WASM module with an infinite loop — should exhaust fuel.
        let infinite_loop_wat = r#"
            (module
              (memory (export "memory") 1)
              (global $bump (mut i32) (i32.const 1024))

              (func (export "alloc") (param $size i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $size)))
                (local.get $ptr)
              )

              (func (export "pattern_execute") (param $ptr i32) (param $len i32) (result i64)
                ;; Infinite loop: consume all fuel
                (loop $inf
                  (br $inf)
                )
                ;; Unreachable — just to satisfy the type checker
                (i64.const 0)
              )
            )
        "#;

        let wasm_bytes = wat::parse_str(infinite_loop_wat).expect("WAT should parse");

        let mut engine_cfg = wasmtime::Config::new();
        engine_cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&engine_cfg).unwrap();
        let module = wasmtime::Module::new(&engine, &wasm_bytes).unwrap();

        let pattern = WasmPattern {
            engine,
            module,
            name: "infinite".into(),
            fuel: 1_000, // Very low fuel — should run out quickly.
        };

        let registry = Arc::new(PatternRegistry::new());
        let ctx = dummy_ctx(&registry);
        let input = serde_json::json!({"hello": "world"});

        let result = pattern.execute(input, ctx).await;
        let err = result.expect_err("should fail due to fuel exhaustion");
        let msg = err.to_string();
        assert!(
            msg.contains("fuel"),
            "error should mention fuel, got: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // 6. WASM echo plugin roundtrip (fresh store per call, no leakage)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_wasm_plugin_echo_roundtrip() {
        // Load the echo.wat fixture.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("echo.wat");

        let wat_bytes = std::fs::read(&fixture)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", fixture.display()));

        let wasm_bytes = wat::parse_bytes(&wat_bytes).expect("WAT fixture should parse");

        let mut engine_cfg = wasmtime::Config::new();
        engine_cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&engine_cfg).unwrap();
        let module = wasmtime::Module::new(&engine, &*wasm_bytes).unwrap();

        let pattern = WasmPattern {
            engine,
            module,
            name: "echo".into(),
            fuel: 100_000,
        };

        let registry = Arc::new(PatternRegistry::new());

        // Execute 3 times with different inputs — proves fresh store per call.
        let inputs = vec![
            serde_json::json!({"round": 1}),
            serde_json::json!({"round": 2, "extra": "data"}),
            serde_json::json!([1, 2, 3]),
        ];

        for input in inputs {
            let ctx = dummy_ctx(&registry);
            let output = pattern
                .execute(input.clone(), ctx)
                .await
                .expect("echo execute should succeed");
            assert_eq!(output, input, "echo pattern should return input unchanged");
        }
    }
}
