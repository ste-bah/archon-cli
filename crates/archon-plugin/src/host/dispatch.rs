use super::WasmRuntime;

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
pub(super) fn call_wasm_tool(
    runtime: &mut WasmRuntime,
    tool_name: &str,
    args_json: &str,
) -> anyhow::Result<String> {
    const RESULT_MAX_BYTES: i32 = 65536;

    // Get required functions. Use `&mut runtime.store` at each site to force a
    // reborrow instead of a move, allowing multiple uses of the store reference.
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

    // Allocate guest memory for name.
    let name_len = name_bytes.len() as i32;
    let name_ptr = alloc.call(&mut runtime.store, name_len)?;

    // Write name into guest memory.
    {
        let mem = memory.data_mut(&mut runtime.store);
        let start = name_ptr as usize;
        let end = start + name_bytes.len();
        if end > mem.len() {
            anyhow::bail!("guest memory too small for tool name");
        }
        mem[start..end].copy_from_slice(name_bytes);
    }

    // Allocate guest memory for args.
    let args_len = args_bytes.len().max(1) as i32; // at least 1 to avoid zero-alloc
    let args_ptr = alloc.call(&mut runtime.store, args_len)?;

    // Write args into guest memory.
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

    let result_ptr = alloc.call(&mut runtime.store, RESULT_MAX_BYTES)?;
    let result_len_ptr = alloc.call(&mut runtime.store, 4)?;

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

    let result_bytes_written = {
        let mem = memory.data(&runtime.store);
        let start = result_len_ptr as usize;
        if start + 4 > mem.len() {
            anyhow::bail!("result_len_ptr out of bounds");
        }
        let len_bytes: [u8; 4] = mem[start..start + 4].try_into().unwrap();
        i32::from_le_bytes(len_bytes) as usize
    };

    let result = {
        let mem = memory.data(&runtime.store);
        let start = result_ptr as usize;
        let end = start + result_bytes_written.min(RESULT_MAX_BYTES as usize);
        if end > mem.len() {
            anyhow::bail!("result_ptr range out of bounds");
        }
        String::from_utf8_lossy(&mem[start..end]).into_owned()
    };

    if serde_json::from_str::<serde_json::Value>(&result).is_ok() {
        Ok(result)
    } else {
        Ok(serde_json::json!({"result": result}).to_string())
    }
}
