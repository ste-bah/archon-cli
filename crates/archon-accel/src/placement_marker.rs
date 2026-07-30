//! Marker-specific placement: per-document VRAM footprint, page-range chunking, and the
//! device env ladder for the surya sidecar.
//!
//! Split out of `placement.rs` to keep both files under the 500-line gate.

use super::*;
/// set; `PER_PAGE` = the OCR encoder's marginal cost per page; `CAP` = the conservative saturation
/// ceiling. Shared by [`marker_footprint_mb`] (pages→MiB) and [`marker_chunk_pages`] (its inverse,
/// MiB→pages, used to size chunks). Keep the three in one place so both directions stay consistent.
const MARKER_FLOOR_MB: u64 = 6000;
const MARKER_PER_PAGE_MB: u64 = 30;
const MARKER_CAP_MB: u64 = 10240;

/// Marker's per-document GPU footprint (MiB), MEASURED (PR-D, marker-pdf 1.10.2 / surya 0.17.1 on
/// RTX 5070 CUDA + Apple MPS). Marker's VRAM is set by the OCR encoder's whole-document pass, so it
/// rises with document size from a ~6 GiB model floor and SATURATES (it is NOT unbounded, and it is
/// independent of batch size): three points — 13pp→5956, 129pp→9424, 578pp→8470 MiB reserved — fit
/// `min(6000 + 30·pages, 10240)` (context folded in). The cap is conservative (the 578pp doc came
/// in under it — page resolution, not count, drives the top end) and the per-doc OOM→CPU fallback
/// backstops any document that still exceeds it. `pages = 0` (unknown) → the ~6 GiB floor.
pub fn marker_footprint_mb(pages: u32) -> u64 {
    (MARKER_FLOOR_MB + MARKER_PER_PAGE_MB * pages as u64).min(MARKER_CAP_MB)
}

/// Inverse of [`marker_footprint_mb`]: the largest page-range chunk whose footprint fits `usable_mb`
/// of free VRAM. `None` if not even the ~6 GiB model floor fits (→ the document can only run on
/// CPU). This is how a big document is kept on a small card — split it into chunks this many pages
/// wide (batch size is VRAM-inert, so page-range is the only real lever, PR-D).
fn marker_chunk_pages(usable_mb: u64) -> Option<u32> {
    if usable_mb <= MARKER_FLOOR_MB {
        return None;
    }
    Some((((usable_mb - MARKER_FLOOR_MB) / MARKER_PER_PAGE_MB).max(1)) as u32)
}

/// Build the Marker sidecar env for a device + surya config (pure). Batch caps are NOT a VRAM lever
/// (PR-D) — GPU is left to surya's own defaults; CPU gets small caps to bound RAM/latency; CUDA
/// always gets `expandable_segments` to cut fragmentation OOM.
pub fn marker_env_for(device: AccelKind, tier: SuryaTier) -> Vec<(String, String)> {
    let mut env = vec![
        (
            "TORCH_DEVICE".to_string(),
            device.sidecar_device().to_string(),
        ),
        (
            "TORCH_DTYPE".to_string(),
            if device == AccelKind::Cpu {
                "float32"
            } else {
                "float16"
            }
            .to_string(),
        ),
    ];
    if matches!(tier, SuryaTier::Cpu) {
        for (k, v) in [
            ("RECOGNITION_BATCH_SIZE", "8"),
            ("DETECTOR_BATCH_SIZE", "4"),
            ("LAYOUT_BATCH_SIZE", "2"),
            ("TABLE_REC_BATCH_SIZE", "4"),
            ("OCR_ERROR_BATCH_SIZE", "4"),
        ] {
            env.push((k.to_string(), v.to_string()));
        }
    }
    if device == AccelKind::Cuda {
        env.push((
            "PYTORCH_CUDA_ALLOC_CONF".to_string(),
            "expandable_segments:True".to_string(),
        ));
    }
    env
}

/// The full plan: one placement per registered consumer, plus the GPU budget that was used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementPlan {
    pub placements: Vec<Placement>,
    pub report_summary: String,
    pub gpu_budget_mb: u64,
}

impl PlacementPlan {
    pub fn get(&self, kind: ConsumerKind) -> Option<&Placement> {
        self.placements.iter().find(|p| p.consumer == kind)
    }

    pub fn marker(&self) -> Option<&Placement> {
        self.get(ConsumerKind::Marker)
    }
}

/// Plan placement for the standard PDF ingest consumers (Marker on GPU-if-it-fits, embedding
/// CPU). Convenience over [`plan_placement`].
pub fn plan_marker_ingest(
    report: &AcceleratorReport,
    overrides: &DeviceOverrides,
    pages: u32,
) -> PlacementPlan {
    let models = ModelFootprintTable::default();
    let consumers = [
        ConsumerRequest::marker(marker_footprint_mb(pages)),
        ConsumerRequest::embedding(models.embedding_mb),
    ];
    plan_placement(report, &consumers, overrides)
}

/// The sidecar env for a forced-CPU Marker fallback (the last rung of the OOM ladder).
pub fn marker_cpu_fallback_env() -> Vec<(String, String)> {
    marker_env_for(AccelKind::Cpu, SuryaTier::Cpu)
}

/// Ordered Marker attempts for one document: `(sidecar_device, env)`. `pages` sizes the footprint,
/// so a big document that won't fit the card starts on CPU. A GPU placement yields two rungs —
/// `[GPU, CPU]` — because a torch-OOM is only relieved by CPU (PR-D: smaller batches don't reduce
/// Marker's VRAM). `archon-docs` runs the list top-to-bottom, advancing only on a torch-OOM.
pub fn marker_env_ladder(
    report: &AcceleratorReport,
    overrides: &DeviceOverrides,
    pages: u32,
) -> Vec<(String, Vec<(String, String)>)> {
    let plan = plan_marker_ingest(report, overrides, pages);
    let m = plan
        .marker()
        .expect("plan_marker_ingest always yields a Marker placement");
    if m.device == AccelKind::Cpu {
        return vec![("cpu".to_string(), marker_cpu_fallback_env())];
    }
    vec![
        (
            m.device.sidecar_device().to_string(),
            marker_env_for(m.device, m.surya_tier),
        ),
        ("cpu".to_string(), marker_cpu_fallback_env()),
    ]
}

/// One page-range unit of a (possibly chunked) Marker ingest. `page_range` is 0-indexed inclusive
/// (`None` = the whole document — no `--page-range` passed to the sidecar). `attempts` is that
/// chunk's own OOM ladder — `(sidecar_device, env)`, GPU then CPU — run top-to-bottom, advancing
/// only on a torch-OOM. Marker emits ABSOLUTE page ids per the requested range, so chunk block
/// streams concatenate directly (no re-offset) into one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerChunk {
    pub page_range: Option<(u32, u32)>,
    pub attempts: Vec<(String, Vec<(String, String)>)>,
}

/// Plan a (possibly chunked) Marker ingest for a `pages`-page document. Three outcomes:
/// - the whole doc fits the GPU (or a device was forced) → a single whole-doc chunk (`page_range:
///   None`) on `[GPU, CPU]`;
/// - the whole doc won't fit the card whole, but a page-range chunk fits → several contiguous GPU
///   chunks each sized (via [`marker_chunk_pages`]) to free VRAM, so a big document still runs on a
///   small card's GPU — page-range chunking is the ONLY lever for this (batch size is VRAM-inert,
///   PR-D);
/// - no GPU, forced CPU, or not even the ~6 GiB floor fits → a single whole-doc CPU chunk.
///
/// A GPU chunk that still OOMs falls to CPU for that chunk (its bboxes then differ by device — an
/// accepted, already-by-design consequence of §8 verify-by-recompute).
pub fn marker_ingest_plan(
    report: &AcceleratorReport,
    overrides: &DeviceOverrides,
    pages: u32,
) -> Vec<MarkerChunk> {
    let whole_doc_cpu = || {
        vec![MarkerChunk {
            page_range: None,
            attempts: vec![("cpu".to_string(), marker_cpu_fallback_env())],
        }]
    };
    let gpu_then_cpu = |kind: AccelKind, range: Option<(u32, u32)>| MarkerChunk {
        page_range: range,
        attempts: vec![
            (
                kind.sidecar_device().to_string(),
                marker_env_for(kind, SuryaTier::Gpu),
            ),
            ("cpu".to_string(), marker_cpu_fallback_env()),
        ],
    };

    let plan = plan_marker_ingest(report, overrides, pages);
    let m = plan
        .marker()
        .expect("plan_marker_ingest always yields a Marker placement");

    // Whole doc already placed on a GPU (it fits, or a GPU was forced) → single whole-doc chunk.
    if m.device != AccelKind::Cpu {
        return vec![gpu_then_cpu(m.device, None)];
    }
    // Whole doc routed to CPU. Only chunking onto a GPU can rescue it — and never when CPU was the
    // user's explicit choice.
    if overrides.force_marker_device == Some(AccelKind::Cpu) {
        return whole_doc_cpu();
    }
    let Some(gpu) = report.best_gpu() else {
        return whole_doc_cpu();
    };
    let usable = plan.gpu_budget_mb.saturating_sub(overrides.headroom_mb);
    let Some(chunk_pages) = marker_chunk_pages(usable) else {
        return whole_doc_cpu(); // not even the model floor fits → CPU
    };
    // Split [0, pages) into contiguous 0-indexed inclusive ranges of `chunk_pages` each.
    let total = pages.max(1);
    let mut chunks = Vec::new();
    let mut start = 0u32;
    while start < total {
        let end = (start + chunk_pages - 1).min(total - 1);
        chunks.push(gpu_then_cpu(gpu.kind, Some((start, end))));
        start = end + 1;
    }
    chunks
}
