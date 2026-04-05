//! Host-guest ABI constants and memory helpers for TASK-CLI-301.

// ── Version constants ─────────────────────────────────────────────────────────

/// The API version this host implements.
pub const HOST_API_VERSION: u32 = 1;

/// Minimum guest API version the host will load (inclusive).
pub const MIN_SUPPORTED_GUEST_VERSION: u32 = 1;

/// Default fuel budget per host function call (≈ 10M instructions).
pub const DEFAULT_FUEL_BUDGET: u64 = 10_000_000;

/// Default maximum memory per plugin instance (64 MiB).
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;

// ── Valid hook event names ────────────────────────────────────────────────────

/// All hook event names accepted by `archon_register_hook`.
///
/// Any event name not in this list is rejected with `PluginError::AbiMismatch`.
pub const VALID_HOOK_EVENTS: &[&str] = &[
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "UserPromptSubmit",
    "SessionStart",
    "SessionEnd",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "PermissionDenied",
    "Notification",
    "PreCompact",
    "PostCompact",
    "ConfigChange",
    "CwdChanged",
    "FileChanged",
    "TaskCreated",
    "TaskCompleted",
    "WorktreeCreate",
    "WorktreeRemove",
];

// ── Memory helpers ────────────────────────────────────────────────────────────

/// Read a UTF-8 string from guest linear memory at `[ptr, ptr+len)`.
///
/// Returns `None` if the range is out of bounds or the bytes are not valid UTF-8.
pub fn read_guest_str(memory_data: &[u8], ptr: i32, len: i32) -> Option<String> {
    let start = usize::try_from(ptr).ok()?;
    let length = usize::try_from(len).ok()?;
    let end = start.checked_add(length)?;
    let bytes = memory_data.get(start..end)?;
    std::str::from_utf8(bytes).ok().map(String::from)
}

/// Write `data` into guest linear memory starting at `ptr`.
///
/// Returns `true` on success, `false` if the write would overflow the buffer.
pub fn write_guest_memory(memory_data: &mut [u8], ptr: i32, data: &[u8]) -> bool {
    let start = match usize::try_from(ptr) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let end = match start.checked_add(data.len()) {
        Some(v) => v,
        None => return false,
    };
    if end > memory_data.len() {
        return false;
    }
    memory_data[start..end].copy_from_slice(data);
    true
}

/// Write a little-endian i32 into guest memory at `ptr`.
pub fn write_guest_i32(memory_data: &mut [u8], ptr: i32, value: i32) -> bool {
    let bytes = value.to_le_bytes();
    write_guest_memory(memory_data, ptr, &bytes)
}
