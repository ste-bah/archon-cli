//! `cargo run -p archon-accel --example probe [pages]` — print the detected accelerator report and
//! the Marker placement for a document of `pages` pages (default 50) on THIS host. Marker's VRAM
//! scales with document size (PR-D), so the placement depends on both the card and the doc.

use archon_accel::{detect, marker_footprint_mb, plan_marker_ingest, DeviceOverrides};

fn main() {
    let pages: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let report = detect();
    println!("== AcceleratorReport ==");
    println!("{}", report.summary());
    for n in &report.notes {
        println!("  note: {n}");
    }

    println!(
        "\n== Marker placement for a {pages}-page doc (footprint {} MiB) ==",
        marker_footprint_mb(pages)
    );
    let plan = plan_marker_ingest(&report, &DeviceOverrides::default(), pages);
    println!("gpu_budget = {} MiB", plan.gpu_budget_mb);
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
