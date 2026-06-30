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

/// surya batch-size tier for the Marker sidecar. The concrete batch numbers in
/// [`Placement::marker_sidecar_env`] are PROVISIONAL — tuned against real surya VRAM peaks
/// in PR-D.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuryaTier {
    Generous,
    Reduced,
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

/// Provisional model footprints (MiB). All UNVERIFIED — Marker ~5 GiB is a reader estimate,
/// not a measurement, and surya peak scales with page complexity. PR-D captures real peaks.
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
            marker_mb: 5120,
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
            headroom_mb: 1536,
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

    fn gpu(
        kind: ConsumerKind,
        device: AccelKind,
        index: u32,
        surya_tier: SuryaTier,
        expandable: bool,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            consumer: kind,
            device,
            device_index: Some(index),
            precision: Precision::Fp16,
            surya_tier,
            cuda_expandable_segments: expandable && device == AccelKind::Cuda,
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
            surya_tier: SuryaTier::Reduced,
            cuda_expandable_segments: device == AccelKind::Cuda,
            oom_fallback_to_cpu: true,
            reason: "forced device via override (OOM-guarded)".to_string(),
        }
    }

    /// Whether this placement runs on a GPU.
    pub fn is_gpu(&self) -> bool {
        self.device != AccelKind::Cpu
    }

    /// Environment the Marker sidecar should run under for this placement. Only meaningful for
    /// the Marker consumer. surya batch sizes are PROVISIONAL — PR-D tunes them against real
    /// VRAM peaks.
    pub fn marker_sidecar_env(&self) -> Vec<(String, String)> {
        let mut env = vec![
            (
                "TORCH_DEVICE".to_string(),
                self.device.sidecar_device().to_string(),
            ),
            (
                "TORCH_DTYPE".to_string(),
                match self.precision {
                    Precision::Fp16 => "float16",
                    Precision::Fp32 => "float32",
                }
                .to_string(),
            ),
        ];
        // (recognition, detector, layout, table_rec, ocr_error) batch caps per tier.
        let caps = match self.surya_tier {
            SuryaTier::Generous => Some((64, 24, 12, 24, 16)),
            SuryaTier::Reduced => Some((16, 8, 4, 8, 6)),
            SuryaTier::Cpu => Some((8, 4, 2, 4, 4)),
            SuryaTier::None => None,
        };
        if let Some((rec, det, lay, tab, ocr)) = caps {
            env.push(("RECOGNITION_BATCH_SIZE".to_string(), rec.to_string()));
            env.push(("DETECTOR_BATCH_SIZE".to_string(), det.to_string()));
            env.push(("LAYOUT_BATCH_SIZE".to_string(), lay.to_string()));
            env.push(("TABLE_REC_BATCH_SIZE".to_string(), tab.to_string()));
            env.push(("OCR_ERROR_BATCH_SIZE".to_string(), ocr.to_string()));
        }
        if self.cuda_expandable_segments {
            env.push((
                "PYTORCH_CUDA_ALLOC_CONF".to_string(),
                "expandable_segments:True".to_string(),
            ));
        }
        env
    }
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
) -> PlacementPlan {
    let models = ModelFootprintTable::default();
    let consumers = [
        ConsumerRequest::marker(models.marker_mb),
        ConsumerRequest::embedding(models.embedding_mb),
    ];
    plan_placement(report, &consumers, overrides)
}

/// The sidecar env for a forced-CPU Marker fallback — used on the per-doc GPU-OOM retry in
/// `archon-docs`. Equivalent to a CPU Marker placement's env (cpu device + small surya caps).
pub fn marker_cpu_fallback_env() -> Vec<(String, String)> {
    Placement::cpu(ConsumerKind::Marker, "gpu-oom fallback").marker_sidecar_env()
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

    let footprint = c.footprint_mb;
    let need = footprint.saturating_add(overrides.headroom_mb);
    let before = *remaining;
    if before >= need {
        *remaining = before - footprint;
        Placement::gpu(
            c.kind,
            gkind,
            gidx,
            SuryaTier::Generous,
            false,
            format!("free {before} MiB >= footprint+headroom {need} MiB"),
        )
    } else if before >= footprint {
        *remaining = before - footprint;
        Placement::gpu(
            c.kind,
            gkind,
            gidx,
            SuryaTier::Reduced,
            true,
            format!("free {before} MiB in [footprint {footprint}, need {need}) -> reduced caps + OOM->CPU"),
        )
    } else {
        Placement::cpu(
            c.kind,
            format!(
                "free {before} MiB < footprint {footprint} MiB -> CPU (co-tenancy/constrained)"
            ),
        )
    }
}
