//! Pure placement planner: fit GPU consumers (Marker now; whisper / frame-VLM later) to the
//! host's *free* memory. Deterministic and host-agnostic — every decision is unit-testable
//! on CPU-only CI runners. The load-bearing rule is `free < footprint → CPU`, proven on the
//! dev box (RTX 5090, 32607 MiB total / ~139 MiB free under co-tenancy). GPU placements
//! ALWAYS carry a per-doc OOM→CPU fallback. See
//! `plans/archon-ingestion-port-finish-plan-2026-06-30.md` §4.3.

use serde::{Deserialize, Serialize};

use crate::report::{AccelKind, AcceleratorReport};

/// Which ingest model wants placement. Marker is the only GPU consumer in the PDF round; the
/// rest are the forward seam for the video port (multi-consumer arbitration).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerKind {
    Marker,
    Embedding,
    Whisper,
    FrameVlm,
}

/// Compute precision requested for a placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    Fp16,
    Fp32,
}

/// surya batch config for the Marker sidecar. PR-D calibration proved batch size does NOT change
/// Marker's VRAM (surya's OCR encoder ingests the whole document in one pass), so this is only a
/// device-appropriate config, not a memory lever: `Gpu` = surya's own defaults (+ CUDA expandable
/// segments), `Cpu` = small caps to bound RAM/latency, `None` = a non-Marker consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuryaTier {
    Gpu,
    Cpu,
    None,
}

/// A registered request for compute from one consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerRequest {
    pub kind: ConsumerKind,
    /// Estimated peak footprint in MiB. UNVERIFIED defaults (PR-D measures real peaks).
    pub footprint_mb: u64,
    pub prefers_gpu: bool,
    pub can_cpu_fallback: bool,
    /// Higher = gets scarce VRAM first when several consumers prefer GPU.
    pub priority: u8,
}

impl ConsumerRequest {
    pub fn marker(footprint_mb: u64) -> Self {
        Self {
            kind: ConsumerKind::Marker,
            footprint_mb,
            prefers_gpu: true,
            can_cpu_fallback: true,
            priority: 100,
        }
    }

    /// CPU-only this round (GPU embedding EP is opt-in, PR-C2).
    pub fn embedding(footprint_mb: u64) -> Self {
        Self {
            kind: ConsumerKind::Embedding,
            footprint_mb,
            prefers_gpu: false,
            can_cpu_fallback: true,
            priority: 50,
        }
    }

    pub fn whisper(footprint_mb: u64) -> Self {
        Self {
            kind: ConsumerKind::Whisper,
            footprint_mb,
            prefers_gpu: true,
            can_cpu_fallback: true,
            priority: 90,
        }
    }

    pub fn frame_vlm(footprint_mb: u64) -> Self {
        Self {
            kind: ConsumerKind::FrameVlm,
            footprint_mb,
            prefers_gpu: true,
            can_cpu_fallback: true,
            priority: 80,
        }
    }
}

/// Model footprints (MiB) for the non-Marker consumers. NOTE: Marker no longer uses `marker_mb` —
/// its footprint is page-scaled ([`marker_footprint_mb`], PR-D); `marker_mb` is kept only as the
/// ~6 GiB small-doc reference. whisper/frame-VLM are estimates (video round).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFootprintTable {
    pub marker_mb: u64,
    pub embedding_mb: u64,
    pub whisper_mb: u64,
    pub frame_vlm_mb: u64,
}

impl Default for ModelFootprintTable {
    fn default() -> Self {
        Self {
            marker_mb: 6144,
            embedding_mb: 512,
            whisper_mb: 2048,
            frame_vlm_mb: 1536,
        }
    }
}

/// Caller overrides. `force_marker_device` honours an explicit user choice (still OOM-guarded);
/// `memory_budget_mb` clamps usable free VRAM; `headroom_mb` is reserved above a model's
/// footprint before a fit counts as "generous".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceOverrides {
    pub force_marker_device: Option<AccelKind>,
    pub memory_budget_mb: Option<u64>,
    pub headroom_mb: u64,
}

impl Default for DeviceOverrides {
    fn default() -> Self {
        Self {
            force_marker_device: None,
            memory_budget_mb: None,
            headroom_mb: 512,
        }
    }
}

/// Where one consumer should run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub consumer: ConsumerKind,
    pub device: AccelKind,
    pub device_index: Option<u32>,
    pub precision: Precision,
    pub surya_tier: SuryaTier,
    pub cuda_expandable_segments: bool,
    /// GPU placements ALWAYS carry a per-doc OOM→CPU fallback — the load-bearing correctness
    /// guarantee, because the footprint estimate is unverified.
    pub oom_fallback_to_cpu: bool,
    pub reason: String,
}

impl Placement {
    fn cpu(kind: ConsumerKind, reason: impl Into<String>) -> Self {
        // Marker still runs surya on CPU (with small batches); other consumers do not.
        let surya_tier = if matches!(kind, ConsumerKind::Marker) {
            SuryaTier::Cpu
        } else {
            SuryaTier::None
        };
        Self {
            consumer: kind,
            device: AccelKind::Cpu,
            device_index: None,
            precision: Precision::Fp32,
            surya_tier,
            cuda_expandable_segments: false,
            oom_fallback_to_cpu: false,
            reason: reason.into(),
        }
    }

    fn gpu(kind: ConsumerKind, device: AccelKind, index: u32, reason: impl Into<String>) -> Self {
        Self {
            consumer: kind,
            device,
            device_index: Some(index),
            precision: Precision::Fp16,
            surya_tier: SuryaTier::Gpu,
            cuda_expandable_segments: device == AccelKind::Cuda,
            oom_fallback_to_cpu: true,
            reason: reason.into(),
        }
    }

    fn forced(kind: ConsumerKind, device: AccelKind, index: Option<u32>) -> Self {
        if device == AccelKind::Cpu {
            return Self::cpu(kind, "forced cpu via override");
        }
        Self {
            consumer: kind,
            device,
            device_index: index,
            precision: Precision::Fp16,
            surya_tier: SuryaTier::Gpu,
            cuda_expandable_segments: device == AccelKind::Cuda,
            oom_fallback_to_cpu: true,
            reason: "forced device via override (OOM-guarded)".to_string(),
        }
    }

    /// Whether this placement runs on a GPU.
    pub fn is_gpu(&self) -> bool {
        self.device != AccelKind::Cpu
    }

    /// Environment the Marker sidecar should run under for this placement (device + surya batch
    /// caps). Only meaningful for the Marker consumer.
    pub fn marker_sidecar_env(&self) -> Vec<(String, String)> {
        marker_env_for(self.device, self.surya_tier)
    }
}

/// Marker's per-document GPU footprint (MiB), MEASURED (PR-D, marker-pdf 1.10.2 / surya 0.17.1 on
/// RTX 5070 CUDA + Apple MPS). Marker's VRAM is set by the OCR encoder's whole-document pass, so it
/// rises with document size from a ~6 GiB model floor and SATURATES (it is NOT unbounded, and it is
/// independent of batch size): three points — 13pp→5956, 129pp→9424, 578pp→8470 MiB reserved — fit
/// `min(6000 + 30·pages, 10240)` (context folded in). The cap is conservative (the 578pp doc came
/// in under it — page resolution, not count, drives the top end) and the per-doc OOM→CPU fallback
/// backstops any document that still exceeds it. `pages = 0` (unknown) → the ~6 GiB floor.
pub fn marker_footprint_mb(pages: u32) -> u64 {
    (6000 + 30 * pages as u64).min(10240)
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

/// Core planner. Packs GPU-preferring consumers (highest priority first) onto the single
/// best-free GPU; anything that does not fit free memory falls to CPU. Pure + deterministic.
pub fn plan_placement(
    report: &AcceleratorReport,
    consumers: &[ConsumerRequest],
    overrides: &DeviceOverrides,
) -> PlacementPlan {
    let best = report.best_gpu();
    let gpu = best.map(|g| (g.kind, g.index));
    let budget = best
        .map(|g| {
            overrides
                .memory_budget_mb
                .map(|b| b.min(g.free_mb))
                .unwrap_or(g.free_mb)
        })
        .unwrap_or(0);
    let mut remaining = budget;

    // Decide in priority order (GPU-preferring first), but emit in the caller's input order.
    let mut order: Vec<usize> = (0..consumers.len()).collect();
    order.sort_by(|&a, &b| {
        let (ca, cb) = (&consumers[a], &consumers[b]);
        cb.prefers_gpu
            .cmp(&ca.prefers_gpu)
            .then(cb.priority.cmp(&ca.priority))
    });

    let mut slots: Vec<Option<Placement>> = vec![None; consumers.len()];
    for &i in &order {
        slots[i] = Some(decide_one(&consumers[i], gpu, &mut remaining, overrides));
    }
    let placements = slots
        .into_iter()
        .map(|s| s.expect("every consumer decided"))
        .collect();

    PlacementPlan {
        placements,
        report_summary: report.summary(),
        gpu_budget_mb: budget,
    }
}

fn decide_one(
    c: &ConsumerRequest,
    gpu: Option<(AccelKind, u32)>,
    remaining: &mut u64,
    overrides: &DeviceOverrides,
) -> Placement {
    // Explicit Marker device override wins (the user knows their box); still OOM-guarded.
    if c.kind == ConsumerKind::Marker {
        if let Some(forced) = overrides.force_marker_device {
            return Placement::forced(c.kind, forced, gpu.map(|(_, i)| i));
        }
    }
    if !c.prefers_gpu {
        return Placement::cpu(c.kind, "CPU-only this round (GPU EP is opt-in, PR-C2)");
    }
    let (gkind, gidx) = match gpu {
        Some(g) if g.0 != AccelKind::Cpu => g,
        _ => return Placement::cpu(c.kind, "no accelerator detected"),
    };

    // Marker's VRAM is bounded by document size, not batch (PR-D). GPU only if the free pool
    // covers the (page-scaled) footprint + headroom; otherwise CPU. The per-doc OOM->CPU fallback
    // backstops any residual under-estimate.
    let footprint = c.footprint_mb;
    let need = footprint.saturating_add(overrides.headroom_mb);
    let before = *remaining;
    if before < need {
        return Placement::cpu(
            c.kind,
            format!("free {before} MiB < need {need} MiB (footprint {footprint}) -> CPU"),
        );
    }
    *remaining = before - footprint;
    Placement::gpu(
        c.kind,
        gkind,
        gidx,
        format!("free {before} MiB >= need {need} MiB (footprint {footprint}) -> GPU"),
    )
}
