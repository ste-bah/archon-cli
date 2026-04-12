// Agent discovery subsystem — file walking, local/remote sources, watchers.

pub mod local;
pub mod remote;
pub mod walker;
pub mod watcher;

pub use local::{LoadReport, LocalDiscoverySource};
pub use remote::{RemoteDiscoverySource, RemoteLoadReport};
pub use walker::{DiscoveredFile, walk_agents_dir};
pub use watcher::FsWatcher;
