use std::path::Path;

use wasmtime::{Caller, Extern, Instance, Linker, Store};

use super::PluginData;
use crate::abi::{
    HOST_API_VERSION, VALID_HOOK_EVENTS, read_guest_str, write_guest_i32, write_guest_memory,
};
use crate::capability::PluginCapability;
use crate::error::PluginError;
use crate::instance::RegisteredTool;

pub(super) fn verify_required_exports(
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

pub(super) fn call_guest_api_version(
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

pub(super) fn register_host_functions(linker: &mut Linker<PluginData>) -> anyhow::Result<()> {
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

fn get_memory(caller: &mut Caller<PluginData>) -> Option<wasmtime::Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(mem)) => Some(mem),
        _ => None,
    }
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
