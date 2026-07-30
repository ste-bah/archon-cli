//! Host-agnostic decision-rule tests for the placement planner. These run on CPU-only CI runners
//! (and on this GPU dev box) identically, because they feed synthetic reports rather than calling
//! `detect()`. Post-PR-D: Marker's VRAM is bounded by *document size* (not batch), so placement
//! keys off a page-scaled footprint and falls back GPU→CPU on OOM.

use archon_accel::{
    marker_env_for, marker_env_ladder, marker_footprint_mb, marker_ingest_plan, plan_marker_ingest,
    plan_placement, AccelKind, Accelerator, AcceleratorReport, ConsumerKind, ConsumerRequest,
    DeviceOverrides, MarkerChunk, ModelFootprintTable, Precision, SuryaTier,
};

fn cuda(total: u64, free: u64) -> AcceleratorReport {
    AcceleratorReport {
        platform: "linux".into(),
        arch: "x86_64".into(),
        accelerators: vec![Accelerator {
            kind: AccelKind::Cuda,
            index: 0,
            name: "NVIDIA RTX".into(),
            total_mb: total,
            free_mb: free,
        }],
        host_ram_total_mb: 64_000,
        host_ram_free_mb: 32_000,
        unified_memory: false,
        notes: vec![],
    }
}

fn metal(total: u64, free: u64) -> AcceleratorReport {
    AcceleratorReport {
        platform: "macos".into(),
        arch: "aarch64".into(),
        accelerators: vec![Accelerator {
            kind: AccelKind::Metal,
            index: 0,
            name: "Apple Silicon (unified)".into(),
            total_mb: total,
            free_mb: free,
        }],
        host_ram_total_mb: total,
        host_ram_free_mb: free,
        unified_memory: true,
        notes: vec![],
    }
}

fn cpu_only() -> AcceleratorReport {
    AcceleratorReport {
        platform: "linux".into(),
        arch: "x86_64".into(),
        accelerators: vec![],
        host_ram_total_mb: 16_000,
        host_ram_free_mb: 8_000,
        unified_memory: false,
        notes: vec!["no accelerator detected; CPU placement".into()],
    }
}

// footprint(13) = 6390 (need 6902); footprint(300) = 10240 cap (need 10752).
const SMALL: u32 = 13;
const LARGE: u32 = 300;

#[test]
fn footprint_scales_with_pages_then_caps() {
    assert_eq!(marker_footprint_mb(0), 6000);
    assert_eq!(marker_footprint_mb(13), 6390);
    assert_eq!(marker_footprint_mb(129), 9870);
    assert_eq!(marker_footprint_mb(300), 10240); // capped
    assert_eq!(marker_footprint_mb(578), 10240); // capped
    assert_eq!(marker_footprint_mb(5000), 10240); // capped, never unbounded
}

#[test]
fn big_card_tiny_free_routes_marker_to_cpu() {
    // Verified dev-box case: RTX 5090, 32607 MiB total but ~139 MiB free under co-tenancy.
    let plan = plan_marker_ingest(&cuda(32_607, 139), &DeviceOverrides::default(), SMALL);
    let m = plan.marker().unwrap();
    assert_eq!(m.device, AccelKind::Cpu, "tiny free must NOT pick GPU");
    assert!(m.reason.contains("< need"), "reason: {}", m.reason);
}

#[test]
fn small_doc_fits_idle_8gb_on_gpu() {
    // 8 GB laptop, idle (~7.8 GB free); a 13pp doc (footprint 6390, need 6902) fits.
    let plan = plan_marker_ingest(&cuda(8_151, 7_822), &DeviceOverrides::default(), SMALL);
    let m = plan.marker().unwrap();
    assert_eq!(m.device, AccelKind::Cuda);
    assert_eq!(m.precision, Precision::Fp16);
    assert_eq!(m.surya_tier, SuryaTier::Gpu);
    assert!(m.oom_fallback_to_cpu);
    assert!(m.cuda_expandable_segments);
}

#[test]
fn large_doc_cpu_on_8gb_but_gpu_on_16gb() {
    // 300pp doc: footprint capped 10240, need 10752.
    let laptop = plan_marker_ingest(&cuda(8_151, 7_822), &DeviceOverrides::default(), LARGE);
    assert_eq!(
        laptop.marker().unwrap().device,
        AccelKind::Cpu,
        "big doc won't fit 8 GB -> CPU (no wasted GPU-OOM cycle)"
    );
    let card16 = plan_marker_ingest(&cuda(16_384, 15_000), &DeviceOverrides::default(), LARGE);
    assert_eq!(card16.marker().unwrap().device, AccelKind::Cuda);
}

#[test]
fn no_gpu_everything_cpu() {
    let plan = plan_marker_ingest(&cpu_only(), &DeviceOverrides::default(), SMALL);
    for p in &plan.placements {
        assert_eq!(p.device, AccelKind::Cpu);
    }
}

#[test]
fn apple_unified_small_doc_on_metal() {
    let plan = plan_marker_ingest(&metal(24_576, 10_000), &DeviceOverrides::default(), SMALL);
    let m = plan.marker().unwrap();
    assert_eq!(m.device, AccelKind::Metal);
    assert_eq!(m.surya_tier, SuryaTier::Gpu);
    assert!(m.oom_fallback_to_cpu);
    assert!(
        !m.cuda_expandable_segments,
        "expandable_segments is CUDA-only"
    );
}

#[test]
fn embedding_is_always_cpu_this_round() {
    let plan = plan_marker_ingest(&cuda(24_000, 20_000), &DeviceOverrides::default(), SMALL);
    assert_eq!(
        plan.get(ConsumerKind::Embedding).unwrap().device,
        AccelKind::Cpu
    );
}

#[test]
fn memory_budget_override_can_force_cpu() {
    let overrides = DeviceOverrides {
        memory_budget_mb: Some(4_000),
        ..DeviceOverrides::default()
    };
    // 20 GB free, but the budget clamps usable to 4 GB < footprint 6390.
    let plan = plan_marker_ingest(&cuda(24_000, 20_000), &overrides, SMALL);
    assert_eq!(plan.marker().unwrap().device, AccelKind::Cpu);
}

#[test]
fn force_marker_cpu_override_honored() {
    let overrides = DeviceOverrides {
        force_marker_device: Some(AccelKind::Cpu),
        ..DeviceOverrides::default()
    };
    let plan = plan_marker_ingest(&cuda(24_000, 20_000), &overrides, SMALL);
    assert_eq!(plan.marker().unwrap().device, AccelKind::Cpu);
}

#[test]
fn force_marker_cuda_honored_even_without_detected_gpu() {
    let overrides = DeviceOverrides {
        force_marker_device: Some(AccelKind::Cuda),
        ..DeviceOverrides::default()
    };
    let plan = plan_marker_ingest(&cpu_only(), &overrides, SMALL);
    let m = plan.marker().unwrap();
    assert_eq!(m.device, AccelKind::Cuda);
    assert!(
        m.oom_fallback_to_cpu,
        "a forced GPU placement is still OOM-guarded"
    );
}

#[test]
fn gpu_env_has_no_batch_caps_but_cpu_does() {
    use std::collections::HashMap;
    let gpu: HashMap<_, _> = marker_env_for(AccelKind::Cuda, SuryaTier::Gpu)
        .into_iter()
        .collect();
    assert_eq!(gpu.get("TORCH_DEVICE").map(String::as_str), Some("cuda"));
    assert_eq!(gpu.get("TORCH_DTYPE").map(String::as_str), Some("float16"));
    assert!(gpu.contains_key("PYTORCH_CUDA_ALLOC_CONF"));
    assert!(
        !gpu.contains_key("RECOGNITION_BATCH_SIZE"),
        "GPU leaves batch to surya defaults (batch is VRAM-inert)"
    );

    let cpu: HashMap<_, _> = marker_env_for(AccelKind::Cpu, SuryaTier::Cpu)
        .into_iter()
        .collect();
    assert_eq!(cpu.get("TORCH_DEVICE").map(String::as_str), Some("cpu"));
    assert!(cpu.contains_key("RECOGNITION_BATCH_SIZE"), "CPU caps batch");
    assert!(!cpu.contains_key("PYTORCH_CUDA_ALLOC_CONF"));
}

#[test]
fn oom_ladder_is_gpu_then_cpu() {
    // GPU placement -> two rungs [gpu, cpu]; smaller batches don't help, so CPU is the only retry.
    let ladder = marker_env_ladder(&cuda(24_000, 20_000), &DeviceOverrides::default(), SMALL);
    assert_eq!(ladder.len(), 2);
    assert_eq!(ladder[0].0, "cuda");
    assert_eq!(ladder[1].0, "cpu");
    // CPU placement -> single rung.
    let cpu = marker_env_ladder(&cpu_only(), &DeviceOverrides::default(), SMALL);
    assert_eq!(cpu.len(), 1);
    assert_eq!(cpu[0].0, "cpu");
}

#[test]
fn multi_consumer_seam_packs_by_priority() {
    // Forward seam (video): Marker(prio 100, ~6390) + Whisper(prio 90, 2048) on 8000 MiB free.
    // Marker fits (need 6902), leaving ~1610 -> Whisper falls to CPU.
    let models = ModelFootprintTable::default();
    let consumers = [
        ConsumerRequest::marker(marker_footprint_mb(SMALL)),
        ConsumerRequest::whisper(models.whisper_mb),
    ];
    let plan = plan_placement(
        &cuda(16_000, 8_000),
        &consumers,
        &DeviceOverrides::default(),
    );
    assert_eq!(
        plan.get(ConsumerKind::Marker).unwrap().device,
        AccelKind::Cuda
    );
    assert_eq!(
        plan.get(ConsumerKind::Whisper).unwrap().device,
        AccelKind::Cpu
    );
}

// ---- page-range chunking (big-doc-on-small-card) ---------------------------------------------

/// Helper: sidecar device of a chunk's first (preferred) attempt.
fn first_dev(c: &MarkerChunk) -> &str {
    c.attempts.first().unwrap().0.as_str()
}

#[test]
fn chunk_plan_small_doc_is_one_whole_gpu_chunk() {
    // 13pp fits an idle 8 GB card whole → a single `None`-range chunk on [GPU, CPU].
    let chunks = marker_ingest_plan(&cuda(8_151, 7_822), &DeviceOverrides::default(), SMALL);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].page_range, None);
    assert_eq!(first_dev(&chunks[0]), "cuda");
    assert_eq!(chunks[0].attempts.last().unwrap().0, "cpu");
}

#[test]
fn chunk_plan_big_doc_on_16gb_is_one_whole_gpu_chunk() {
    // 300pp (footprint 10240) fits a 16 GB card whole → still a single chunk (no need to split).
    let chunks = marker_ingest_plan(&cuda(16_384, 15_000), &DeviceOverrides::default(), LARGE);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].page_range, None);
    assert_eq!(first_dev(&chunks[0]), "cuda");
}

#[test]
fn chunk_plan_big_doc_on_8gb_splits_into_contiguous_gpu_chunks() {
    // 300pp won't fit an 8 GB card whole (need 10752 > 7310 usable) → split. usable = 7822-512
    // = 7310 → chunk_pages = (7310-6000)/30 = 43 → ceil(300/43) = 7 chunks, each on GPU.
    let chunks = marker_ingest_plan(&cuda(8_151, 7_822), &DeviceOverrides::default(), LARGE);
    assert_eq!(chunks.len(), 7);
    // Every chunk is a real page-range on the GPU ladder, and each fits the card.
    for c in &chunks {
        let (s, e) = c.page_range.expect("chunked → Some(range)");
        assert_eq!(first_dev(c), "cuda");
        assert_eq!(c.attempts.last().unwrap().0, "cpu");
        let pages_in_chunk = e - s + 1;
        assert!(
            marker_footprint_mb(pages_in_chunk) + 512 <= 7_822,
            "chunk {s}..={e} ({pages_in_chunk}pp) must fit the 8 GB card"
        );
    }
    // Ranges tile [0, 299] with no gaps or overlaps.
    assert_eq!(chunks[0].page_range.unwrap().0, 0);
    assert_eq!(chunks.last().unwrap().page_range.unwrap().1, LARGE - 1);
    for w in chunks.windows(2) {
        assert_eq!(w[0].page_range.unwrap().1 + 1, w[1].page_range.unwrap().0);
    }
}

#[test]
fn chunk_plan_tiny_free_is_one_cpu_chunk() {
    // Co-tenanted 5090 (139 MiB free): can't fit even the 6 GiB floor → whole doc on CPU.
    let chunks = marker_ingest_plan(&cuda(32_607, 139), &DeviceOverrides::default(), LARGE);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].page_range, None);
    assert_eq!(first_dev(&chunks[0]), "cpu");
}

#[test]
fn chunk_plan_no_gpu_is_one_cpu_chunk() {
    let chunks = marker_ingest_plan(&cpu_only(), &DeviceOverrides::default(), LARGE);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].page_range, None);
    assert_eq!(first_dev(&chunks[0]), "cpu");
}

#[test]
fn chunk_plan_forced_cpu_never_chunks_to_gpu() {
    // Even with a big idle GPU, forcing CPU keeps the whole doc on CPU (no page-range split).
    let overrides = DeviceOverrides {
        force_marker_device: Some(AccelKind::Cpu),
        ..DeviceOverrides::default()
    };
    let chunks = marker_ingest_plan(&cuda(24_000, 20_000), &overrides, LARGE);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].page_range, None);
    assert_eq!(first_dev(&chunks[0]), "cpu");
}

#[test]
fn chunk_plan_apple_big_doc_splits_on_metal() {
    // Apple unified, 10 GB free of a 24 GB box, 300pp: won't fit whole (need 10752) → split onto
    // Metal (mps), proving chunking is device-agnostic.
    let chunks = marker_ingest_plan(&metal(24_576, 10_000), &DeviceOverrides::default(), LARGE);
    assert!(chunks.len() > 1);
    assert!(chunks.iter().all(|c| first_dev(c) == "mps"));
    assert_eq!(chunks.last().unwrap().page_range.unwrap().1, LARGE - 1);
}
