//! `cargo run -p archon-accel --example probe` — print the detected accelerator report and
//! the Marker placement plan for THIS host. A quick way to see what the adaptive layer would
//! decide before it is wired into ingest (PR-C).

use archon_accel::{detect, plan_marker_ingest, DeviceOverrides};

fn main() {
    let report = detect();
    println!("== AcceleratorReport ==");
    println!("{}", report.summary());
    for n in &report.notes {
        println!("  note: {n}");
    }

    let plan = plan_marker_ingest(&report, &DeviceOverrides::default());
    println!(
        "\n== PlacementPlan (gpu_budget = {} MiB) ==",
        plan.gpu_budget_mb
    );
    for p in &plan.placements {
        println!(
            "  {:?}: device={} idx={:?} precision={:?} tier={:?} oom_fallback={} expandable={}",
            p.consumer,
            p.device,
            p.device_index,
            p.precision,
            p.surya_tier,
            p.oom_fallback_to_cpu,
            p.cuda_expandable_segments,
        );
        println!("      reason: {}", p.reason);
    }

    if let Some(m) = plan.marker() {
        println!("\n== Marker sidecar env (for this placement) ==");
        for (k, v) in m.marker_sidecar_env() {
            println!("  {k}={v}");
        }
    }
}
