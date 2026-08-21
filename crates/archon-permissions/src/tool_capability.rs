//! #201 Phase 3: the capability class a tool declares about itself.
//!
//! Lives here rather than in `archon-tools` because
//! [`SandboxBackend::check`](crate::sandbox::SandboxBackend::check) is what
//! decides on it, and `archon-permissions` is the leaf crate both the tool
//! layer and the backend layer already share.

/// Where a tool's effects land, and therefore whether an isolation backend can
/// host it.
///
/// The backends used to gate on tool *name*, behind a catch-all that denied
/// everything unlisted. A tool added anywhere in the workspace was then
/// unusable under every backend until three separate match arms were updated,
/// and nothing failed to say so — the terminal tools from #190 shipped broken
/// that way. A class declared by the tool travels with it instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCapability {
    /// Must execute in the active execution world — the tree `Bash` runs
    /// against. Under a real backend that world is not the host, so how the
    /// tool reaches it decides whether the backend can serve it.
    WorldBound(WorldReach),
    /// Touches only Archon's own state: its config, memory, task store, board,
    /// session transcripts. Correct wherever the world lives.
    HostLocal,
    /// Leaves the machine.
    Egress,
    /// Spawns or schedules work: subagents, tasks, teams, cron entries.
    ControlPlane,
}

/// How a world-bound tool reaches the world.
///
/// The class alone cannot allow `Read` while refusing `Write`, and that
/// distinction has to survive: a sandbox that mutates the host tree while
/// claiming isolation is worse than one that refuses to mutate at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorldReach {
    /// Runs a process through
    /// [`SandboxBackend::execute_bash`](crate::sandbox::SandboxBackend::execute_bash),
    /// the one seam every backend implements, so the work lands in the world.
    Execution,
    /// Reads the world's files, through `ToolContext::fs`.
    FileRead,
    /// Writes the world's files, through `ToolContext::fs`.
    FileWrite,
    /// Reaches the world through a host handle no backend can redirect: a host
    /// PTY, a host language server, a subprocess spawned directly rather than
    /// through the execution seam.
    HostHandle,
}

impl ToolCapability {
    /// Shorthand for the common world-bound cases, so a tool declaration reads
    /// as one call rather than two nested constructors.
    pub const EXECUTION: Self = Self::WorldBound(WorldReach::Execution);
    pub const FILE_READ: Self = Self::WorldBound(WorldReach::FileRead);
    pub const FILE_WRITE: Self = Self::WorldBound(WorldReach::FileWrite);
    pub const HOST_HANDLE: Self = Self::WorldBound(WorldReach::HostHandle);

    /// A short stable label, for logs and denial messages.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::WorldBound(WorldReach::Execution) => "world-bound/execution",
            Self::WorldBound(WorldReach::FileRead) => "world-bound/file-read",
            Self::WorldBound(WorldReach::FileWrite) => "world-bound/file-write",
            Self::WorldBound(WorldReach::HostHandle) => "world-bound/host-handle",
            Self::HostLocal => "host-local",
            Self::Egress => "egress",
            Self::ControlPlane => "control-plane",
        }
    }
}

#[cfg(test)]
#[path = "tool_capability_tests.rs"]
mod tests;
