//! The runtime accelerator + free-memory report.
//!
//! `*_mb` fields are mebibytes (MiB) as reported by the source: `nvidia-smi` reports MiB;
//! Apple unified memory is derived from physical RAM. Driven by *free*, not total — a big
//! card under co-tenancy can be effectively unusable (see `best_gpu`).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Class of compute device. Mirrors `BackendKind` from `archon-world-model::backend` but
/// without the `Auto` planning sentinel (planning happens in [`crate::placement`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccelKind {
    Cuda,
    Metal,
    Cpu,
}

impl AccelKind {
    /// The device string the Marker sidecar (`scripts/archon_marker_sidecar.py`) expects for
    /// `TORCH_DEVICE` / `--device`: CUDA→`cuda`, Apple Metal→`mps`, CPU→`cpu`.
    pub fn sidecar_device(self) -> &'static str {
        match self {
            AccelKind::Cuda => "cuda",
            AccelKind::Metal => "mps",
            AccelKind::Cpu => "cpu",
        }
    }
}

impl fmt::Display for AccelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AccelKind::Cuda => "cuda",
            AccelKind::Metal => "metal",
            AccelKind::Cpu => "cpu",
        })
    }
}

/// A single detected accelerator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accelerator {
    pub kind: AccelKind,
    pub index: u32,
    pub name: String,
    pub total_mb: u64,
    pub free_mb: u64,
}

/// Snapshot of the host's compute + memory envelope at ingest time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceleratorReport {
    pub platform: String,
    pub arch: String,
    pub accelerators: Vec<Accelerator>,
    pub host_ram_total_mb: u64,
    pub host_ram_free_mb: u64,
    /// True on Apple Silicon: GPU and CPU share one memory pool, so `total_mb` is physical
    /// RAM and aggressive co-residency causes OS memory *pressure* (soft), not a catchable
    /// CUDA-style OOM.
    pub unified_memory: bool,
    /// Degrade reasons (e.g. `nvidia-smi` missing). Non-fatal; informational.
    pub notes: Vec<String>,
}

impl AcceleratorReport {
    /// The GPU with the most *free* memory (CUDA or Metal), if any. The planner packs onto
    /// this one. Free, not total: a 32 GB card with 139 MB free must route work to CPU.
    pub fn best_gpu(&self) -> Option<&Accelerator> {
        self.accelerators
            .iter()
            .filter(|a| a.kind != AccelKind::Cpu)
            .max_by_key(|a| a.free_mb)
    }

    pub fn has_gpu(&self) -> bool {
        self.best_gpu().is_some()
    }

    /// One-line human summary for logs.
    pub fn summary(&self) -> String {
        let accel = if self.accelerators.is_empty() {
            "cpu-only".to_string()
        } else {
            self.accelerators
                .iter()
                .map(|a| {
                    format!(
                        "{}#{} {} ({} MiB free / {} MiB total)",
                        a.kind, a.index, a.name, a.free_mb, a.total_mb
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "{}/{} | accel: {} | ram {} MiB free / {} MiB total{}",
            self.platform,
            self.arch,
            accel,
            self.host_ram_free_mb,
            self.host_ram_total_mb,
            if self.unified_memory {
                " | unified"
            } else {
                ""
            }
        )
    }
}
