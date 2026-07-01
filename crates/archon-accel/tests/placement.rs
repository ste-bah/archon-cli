//! Host-agnostic decision-rule tests for the placement planner. These run on CPU-only CI
//! runners (and on this GPU dev box) identically, because they feed synthetic reports rather
//! than calling `detect()`.

use archon_accel::{
    plan_marker_ingest, plan_placement, AccelKind, Accelerator, AcceleratorReport, ConsumerKind,
    ConsumerRequest, DeviceOverrides, ModelFootprintTable, Precision, SuryaTier,
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

#[test]
fn big_card_tiny_free_routes_marker_to_cpu() {
    // THE verified dev-box case: RTX 5090, 32607 MiB total but ~139 MiB free under co-tenancy.
    let plan = plan_marker_ingest(&cuda(32_607, 139), &DeviceOverrides::default());
    let m = plan.marker().unwrap();
    assert_eq!(
        m.device,
        AccelKind::Cpu,
        "big total + tiny free must NOT pick GPU"
    );
    assert!(m.reason.contains("< footprint"), "reason: {}", m.reason);
}

#[test]
fn ample_cuda_places_marker_generous_fp16() {
    // 10 GB free -> Generous (>= need 7680, < AMPLE 12288).
    let plan = plan_marker_ingest(&cuda(24_000, 10_000), &DeviceOverrides::default());
    let m = plan.marker().unwrap();
    assert_eq!(m.device, AccelKind::Cuda);
    assert_eq!(m.precision, Precision::Fp16);
    assert_eq!(m.surya_tier, SuryaTier::Generous);
    assert!(m.oom_fallback_to_cpu);
    assert!(!m.cuda_expandable_segments);
}

#[test]
fn constrained_cuda_reduced_with_expandable_and_oom() {
    // footprint 6144, need 6144+1536=7680; free 6500 sits in [footprint, need).
    let plan = plan_marker_ingest(&cuda(8_000, 6_500), &DeviceOverrides::default());
    let m = plan.marker().unwrap();
    assert_eq!(m.device, AccelKind::Cuda);
    assert_eq!(m.surya_tier, SuryaTier::Reduced);
    assert!(m.cuda_expandable_segments);
    assert!(m.oom_fallback_to_cpu);
}

#[test]
fn free_below_footprint_falls_to_cpu() {
    let plan = plan_marker_ingest(&cuda(8_000, 3_000), &DeviceOverrides::default());
    assert_eq!(plan.marker().unwrap().device, AccelKind::Cpu);
}

#[test]
fn no_gpu_everything_cpu() {
    let plan = plan_marker_ingest(&cpu_only(), &DeviceOverrides::default());
    for p in &plan.placements {
        assert_eq!(p.device, AccelKind::Cpu);
    }
}

#[test]
fn apple_unified_ample_places_marker_on_metal() {
    // Mac, ~10 GB free after the OS reserve -> Generous on Metal.
    let plan = plan_marker_ingest(&metal(24_576, 10_000), &DeviceOverrides::default());
    let m = plan.marker().unwrap();
    assert_eq!(m.device, AccelKind::Metal);
    assert_eq!(m.surya_tier, SuryaTier::Generous);
    assert!(m.oom_fallback_to_cpu);
    assert!(
        !m.cuda_expandable_segments,
        "expandable_segments is CUDA-only"
    );
}

#[test]
fn embedding_is_always_cpu_this_round() {
    let plan = plan_marker_ingest(&cuda(24_000, 20_000), &DeviceOverrides::default());
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
    // 20 GB free, but the budget clamps usable to 4 GB < footprint 6144.
    let plan = plan_marker_ingest(&cuda(24_000, 20_000), &overrides);
    assert_eq!(plan.marker().unwrap().device, AccelKind::Cpu);
}

#[test]
fn force_marker_cpu_override_honored() {
    let overrides = DeviceOverrides {
        force_marker_device: Some(AccelKind::Cpu),
        ..DeviceOverrides::default()
    };
    let plan = plan_marker_ingest(&cuda(24_000, 20_000), &overrides);
    assert_eq!(plan.marker().unwrap().device, AccelKind::Cpu);
}

#[test]
fn force_marker_cuda_honored_even_without_detected_gpu() {
    let overrides = DeviceOverrides {
        force_marker_device: Some(AccelKind::Cuda),
        ..DeviceOverrides::default()
    };
    let plan = plan_marker_ingest(&cpu_only(), &overrides);
    let m = plan.marker().unwrap();
    assert_eq!(m.device, AccelKind::Cuda);
    assert!(
        m.oom_fallback_to_cpu,
        "a forced GPU placement is still OOM-guarded"
    );
}

#[test]
fn marker_env_reflects_device_and_tier() {
    let plan = plan_marker_ingest(&cuda(8_000, 6_500), &DeviceOverrides::default());
    let env: std::collections::HashMap<_, _> = plan
        .marker()
        .unwrap()
        .marker_sidecar_env()
        .into_iter()
        .collect();
    assert_eq!(env.get("TORCH_DEVICE").map(String::as_str), Some("cuda"));
    assert_eq!(env.get("TORCH_DTYPE").map(String::as_str), Some("float16"));
    assert!(env.contains_key("RECOGNITION_BATCH_SIZE"));
    assert_eq!(
        env.get("PYTORCH_CUDA_ALLOC_CONF").map(String::as_str),
        Some("expandable_segments:True")
    );
}

#[test]
fn multi_consumer_seam_packs_by_priority() {
    // Forward seam (video): Marker(prio 100, 6144) + Whisper(prio 90, 2048) on 7500 MiB free.
    // Marker fits its footprint (Reduced), leaving ~1356 -> Whisper falls to CPU.
    let models = ModelFootprintTable::default();
    let consumers = [
        ConsumerRequest::marker(models.marker_mb),
        ConsumerRequest::whisper(models.whisper_mb),
    ];
    let plan = plan_placement(&cuda(8_000, 7_500), &consumers, &DeviceOverrides::default());
    assert_eq!(
        plan.get(ConsumerKind::Marker).unwrap().device,
        AccelKind::Cuda
    );
    assert_eq!(
        plan.get(ConsumerKind::Whisper).unwrap().device,
        AccelKind::Cpu
    );
}

#[test]
fn tiers_scale_up_with_free_vram() {
    // Bigger free VRAM -> bigger batch tier (the upward adaptation).
    let gen = plan_marker_ingest(&cuda(24_000, 10_000), &DeviceOverrides::default());
    assert_eq!(gen.marker().unwrap().surya_tier, SuryaTier::Generous);
    let ample = plan_marker_ingest(&cuda(24_000, 15_000), &DeviceOverrides::default());
    assert_eq!(ample.marker().unwrap().surya_tier, SuryaTier::Ample); // 16 GB-class
    let max = plan_marker_ingest(&cuda(32_000, 30_000), &DeviceOverrides::default());
    assert_eq!(max.marker().unwrap().surya_tier, SuryaTier::Max); // 24 GB+ card
}

#[test]
fn bigger_tier_uses_bigger_recognition_batch() {
    let rec = |t| {
        archon_accel::marker_env_for(AccelKind::Cuda, t)
            .into_iter()
            .find(|(k, _)| k == "RECOGNITION_BATCH_SIZE")
            .map(|(_, v)| v.parse::<u32>().unwrap())
            .unwrap()
    };
    assert!(rec(SuryaTier::Max) > rec(SuryaTier::Ample));
    assert!(rec(SuryaTier::Ample) > rec(SuryaTier::Generous));
    assert!(rec(SuryaTier::Generous) > rec(SuryaTier::Reduced));
}

#[test]
fn oom_ladder_steps_down_gpu_then_cpu() {
    // Ample start -> ladder = [cuda Ample, cuda Generous, cuda Reduced, cpu].
    let ladder =
        archon_accel::marker_env_ladder(&cuda(24_000, 15_000), &DeviceOverrides::default());
    assert!(ladder.len() >= 2);
    assert_eq!(ladder.first().unwrap().0, "cuda", "starts on GPU");
    assert_eq!(ladder.last().unwrap().0, "cpu", "ends on CPU last resort");
    // The GPU rungs shrink the recognition batch monotonically.
    let recs: Vec<u32> = ladder
        .iter()
        .filter(|(dev, _)| dev == "cuda")
        .map(|(_, env)| {
            env.iter()
                .find(|(k, _)| k == "RECOGNITION_BATCH_SIZE")
                .map(|(_, v)| v.parse::<u32>().unwrap())
                .unwrap()
        })
        .collect();
    assert!(
        recs.windows(2).all(|w| w[0] > w[1]),
        "GPU rungs step down: {recs:?}"
    );
}

#[test]
fn oom_ladder_cpu_only_when_no_gpu() {
    let ladder = archon_accel::marker_env_ladder(&cpu_only(), &DeviceOverrides::default());
    assert_eq!(ladder.len(), 1);
    assert_eq!(ladder[0].0, "cpu");
}
