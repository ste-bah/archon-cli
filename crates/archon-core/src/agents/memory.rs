//! Agent-scoped memory operations using CozoDB tag-based isolation.
//!
//! This module keeps the historical `agents::memory::*` surface while
//! implementation details live in focused submodules.

mod extraction;
mod file_prompt;
mod meta;
mod store;
mod tags;

pub use extraction::{ExtractionState, has_memory_writes_since, scan_memory_files, should_extract};
pub use file_prompt::{
    MAX_ENTRYPOINT_BYTES, MAX_ENTRYPOINT_LINES, build_full_memory_prompt, ensure_memory_dir_exists,
    get_agent_memory_dir, load_agent_memory_prompt, truncate_entrypoint_content,
};
pub use meta::increment_invocation_count;
pub use store::{load_agent_memory, save_agent_memory};
pub use tags::{agent_tag, scope_tag};

#[cfg(test)]
mod tests;
