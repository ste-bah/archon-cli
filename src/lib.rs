#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::doc_overindented_list_items)]
#![allow(clippy::empty_line_after_doc_comments)]

//! Library target for the archon-cli-workspace root crate.
//!
//! Exposes `cli_args` so integration tests can verify clap parsing
//! without depending on the binary entry point.

pub mod cli_args;

#[cfg(test)]
pub(crate) mod command {
    #[path = "../command/trading_io.rs"]
    pub(crate) mod trading_io;

    #[path = "../command/trading_openbb.rs"]
    pub(crate) mod trading_openbb;

    pub(crate) mod trading_tools {
        use anyhow::Result;
        use std::path::{Path, PathBuf};

        pub(crate) fn project_root(target: Option<&PathBuf>) -> Result<PathBuf> {
            Ok(target.cloned().unwrap_or(std::env::current_dir()?))
        }

        pub(crate) fn openbb_bin(project_root: &Path, name: &str) -> PathBuf {
            openbb_venv_dir(project_root)
                .join(if cfg!(windows) { "Scripts" } else { "bin" })
                .join(name)
        }

        pub(crate) fn openbb_venv_dir(project_root: &Path) -> PathBuf {
            project_root.join(".archon/tools/openbb-venv")
        }
    }
}

pub mod event_coalescer;
