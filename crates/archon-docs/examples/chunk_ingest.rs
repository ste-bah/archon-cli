//! Force a chunked Marker ingest on real hardware and verify the concatenated block stream.
//!
//! ```text
//! cargo run -p archon-docs --example chunk_ingest -- <pdf> <sidecar.py> <python> <budget_mb> <pages> [device]
//! ```
//!
//! With a small `budget_mb` the planner cannot fit the whole document on the GPU, so
//! [`archon_accel::marker_ingest_plan`] carves `pages` into several page-range chunks;
//! [`MarkerSource::blocks_for`] runs each chunk through the sidecar (on the auto-detected GPU) and
//! concatenates the parsed blocks. We then check the blocks span the pages 0..`pages`-1 — the
//! end-to-end proof that page-range chunking keeps a big document on a small card's GPU.
//!
//! `device` defaults to `auto` on purpose: forcing a device pins Marker to a single whole-doc run,
//! which would defeat the split. Leave it `auto` so the budget drives the chunking.

use std::path::Path;

use archon_accel::{DeviceOverrides, detect, marker_ingest_plan};
use archon_docs::marker_source::{MarkerSource, from_policy};
use archon_policy::PdfPolicy;

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        eprintln!(
            "usage: chunk_ingest <pdf> <sidecar.py> <python> <budget_mb> <pages> [device=auto]"
        );
        std::process::exit(2);
    }
    let pdf = a[1].clone();
    let sidecar = a[2].clone();
    let python = a[3].clone();
    let budget: u64 = a[4].parse().expect("budget_mb must be an integer");
    let pages: u32 = a[5].parse().expect("pages must be an integer");
    let device = a.get(6).cloned().unwrap_or_else(|| "auto".to_string());

    // 1) Show the plan the accelerator layer produces for this budget + document size.
    let overrides = DeviceOverrides {
        memory_budget_mb: Some(budget),
        ..Default::default()
    };
    let plan = marker_ingest_plan(&detect(), &overrides, pages);
    println!(
        "== plan: {} chunk(s), budget {budget} MiB, {pages} pages ==",
        plan.len()
    );
    for (i, c) in plan.iter().enumerate() {
        println!(
            "  chunk {i}: page_range={:?} device={}",
            c.page_range, c.attempts[0].0
        );
    }

    // 2) Build the REAL MarkerSource from policy and run the chunked ingest end to end.
    let policy = PdfPolicy {
        marker_sidecar: Some(sidecar),
        marker_python: Some(python),
        marker_memory_budget_mb: Some(budget),
        marker_device: Some(device),
        ..Default::default()
    };
    let src = from_policy(&policy, pages).expect("policy has a sidecar path");
    let planned = match &src {
        MarkerSource::Subprocess { chunks, .. } => chunks.len(),
        _ => 0,
    };
    println!("== running blocks_for over {planned} chunk(s) ==");
    let blocks = src
        .blocks_for(Path::new(&pdf))
        .await
        .expect("blocks_for failed");

    // 3) Verify: the concatenated stream spans the requested pages.
    let mut seen: Vec<u32> = blocks.iter().map(|b| b.page).collect();
    seen.sort_unstable();
    seen.dedup();
    let min = seen.first().copied().unwrap_or(0);
    let max = seen.last().copied().unwrap_or(0);
    println!(
        "== result: {} blocks across {} distinct page(s), span {min}..={max} ==",
        blocks.len(),
        seen.len()
    );
    // `Block.page` is 1-based (the parser maps marker `/page/N/` -> page N+1), so verify by a
    // base-agnostic invariant: `pages` distinct page numbers that are contiguous with no gap.
    let distinct = seen.len() as u32;
    let contiguous = distinct == max.saturating_sub(min) + 1;
    if distinct == pages && contiguous {
        println!(
            "OK: {pages} contiguous pages ({min}..={max}) via {planned}-chunk GPU concat — no gaps"
        );
    } else {
        eprintln!(
            "MISMATCH: expected {pages} contiguous pages; got {distinct} distinct in {min}..={max}"
        );
        std::process::exit(1);
    }
}
