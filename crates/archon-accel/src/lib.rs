//! Archon accelerator + resource-adaptive placement (Port #2 device layer).
//!
//! Two responsibilities, deliberately split:
//!
//! 1. [`detect`] — a RUNTIME probe of the host's accelerators and *free* memory. Unlike
//!    `archon-world-model::backend` (compile-feature-gated, memory-blind), this runs at
//!    startup so a single binary adapts to whatever host it lands on (CUDA laptop, Apple
//!    unified-memory Mac, CPU-only CI). It is degrade-safe: it never panics and always
//!    yields a valid report (CPU-only at worst).
//!
//! 2. [`plan_placement`] / [`plan_marker_ingest`] — a PURE planner that fits GPU consumers
//!    (the Marker sidecar now; whisper / frame-VLM later) to free VRAM. The load-bearing
//!    rule is `free < footprint → CPU` — proven on the dev box (RTX 5090, 32607 MiB total
//!    but ~139 MiB free under co-tenancy). GPU placements ALWAYS carry a per-doc OOM→CPU
//!    fallback, the real correctness guarantee given footprint estimates are unverified.
//!
//! On the PDF path the only GPU consumer is Marker (the embedder is CPU-only this round),
//! so multi-consumer arbitration is a dormant seam — exercised by exactly one consumer now,
//! filled in when the video port registers whisper + frame-VLM.
//!
//! See `plans/archon-ingestion-port-finish-plan-2026-06-30.md` §4.

pub mod detect;
pub mod placement;
pub mod report;

pub use detect::detect;
pub use placement::{
    marker_cpu_fallback_env, marker_env_for, marker_env_ladder, marker_footprint_mb,
    plan_marker_ingest, plan_placement, ConsumerKind, ConsumerRequest, DeviceOverrides,
    ModelFootprintTable, Placement, PlacementPlan, Precision, SuryaTier,
};
pub use report::{AccelKind, Accelerator, AcceleratorReport};
