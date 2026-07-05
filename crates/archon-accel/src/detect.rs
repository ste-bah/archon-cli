//! Runtime detection of accelerators + free memory. Degrade-safe: every failure path yields
//! a valid report (CPU-only at worst); this function never panics.

use crate::report::{AccelKind, Accelerator, AcceleratorReport};

const BYTES_PER_MIB: u64 = 1024 * 1024;

// The Apple constants + budget helper below are only *called* from the macos/aarch64 detect path,
// but stay un-gated so the pure helper unit-tests run on every platform.
#[allow(dead_code)]
const APPLE_OS_RESERVE_MB: u64 = 6144;

/// MEASURED ollama qwen2.5vl:7b footprint on Metal (6.8GB, observed on the 24GB Mac validation
/// run 2026-07-04). Reserved because ollama keeps the VLM model resident across documents
/// (idle-timeout), so Marker's surya and the VLM are potentially coresident in the SAME unified
/// pool — Marker must not claim memory the VLM needs.
#[allow(dead_code)]
const APPLE_VLM_CORESIDENT_RESERVE_MB: u64 = 6800;

/// Unified-memory GPU budget for Apple Silicon (MiB). On unified memory the GPU grows into the
/// shared RAM pool, so the instantaneous free-RAM snapshot understates what Marker's surya model
/// can use. Take the LARGER of (a) instantaneous free minus the OS reserve — right when RAM is
/// plentiful — and (b) the total pool minus OS + a coresident-VLM reserve — right when free is
/// transiently low on a large machine. The `unified` term is free-INDEPENDENT, so on a large
/// machine under heavy non-VLM pressure it can over-report; a clean MPS OOM (`RuntimeError:
/// MPS backend out of memory`) is caught by the per-doc OOM->CPU ladder, but — per
/// [`AcceleratorReport::unified_memory`] — unified over-commit usually shows up as *soft* OS
/// pressure, and a jetsam SIGKILL is NOT ladder-catchable (no exit 42 / OOM stderr). Bounding
/// the `unified` term to a fraction of free + treating signal-kills as CPU-retryable is a
/// tracked hardening follow-up; the current form is validated for the normal (uncontended)
/// bulk-ingest envelope.
#[allow(dead_code)]
fn apple_unified_gpu_budget_mb(ram_total_mb: u64, ram_free_mb: u64) -> u64 {
    let instantaneous = ram_free_mb.saturating_sub(APPLE_OS_RESERVE_MB);
    let unified = ram_total_mb.saturating_sub(APPLE_OS_RESERVE_MB + APPLE_VLM_CORESIDENT_RESERVE_MB);
    instantaneous.max(unified)
}

/// Probe the host. Shell-outs (`nvidia-smi`) or platform calls that fail are recorded in
/// `notes` and degrade to fewer accelerators — never an error.
pub fn detect() -> AcceleratorReport {
    let (ram_total_mb, ram_free_mb) = host_ram();
    let mut notes = Vec::new();
    let accelerators = detect_accelerators(ram_total_mb, ram_free_mb, &mut notes);
    if accelerators.is_empty() {
        notes.push("no accelerator detected; CPU placement".to_string());
    }
    AcceleratorReport {
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        accelerators,
        host_ram_total_mb: ram_total_mb,
        host_ram_free_mb: ram_free_mb,
        unified_memory: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        notes,
    }
}

fn host_ram() -> (u64, u64) {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    (
        sys.total_memory() / BYTES_PER_MIB,
        sys.available_memory() / BYTES_PER_MIB,
    )
}

#[cfg(not(target_os = "macos"))]
fn detect_accelerators(
    _ram_total_mb: u64,
    _ram_free_mb: u64,
    notes: &mut Vec<String>,
) -> Vec<Accelerator> {
    match nvidia_smi() {
        Ok(gpus) => gpus,
        Err(e) => {
            notes.push(format!("cuda: {e}"));
            Vec::new()
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn detect_accelerators(
    ram_total_mb: u64,
    ram_free_mb: u64,
    _notes: &mut Vec<String>,
) -> Vec<Accelerator> {
    // Apple Silicon always exposes a Metal-capable integrated GPU sharing the unified pool.
    // Usable GPU memory ≈ available RAM minus an OS reserve; pressure (not OOM) governs the rest.
    let free_mb = apple_unified_gpu_budget_mb(ram_total_mb, ram_free_mb);
    vec![Accelerator {
        kind: AccelKind::Metal,
        index: 0,
        name: "Apple Silicon (unified)".to_string(),
        total_mb: ram_total_mb,
        free_mb,
    }]
}

#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
fn detect_accelerators(
    _ram_total_mb: u64,
    _ram_free_mb: u64,
    _notes: &mut Vec<String>,
) -> Vec<Accelerator> {
    // Intel Macs: no first-class Metal-compute path wired here → CPU.
    Vec::new()
}

/// Parse `nvidia-smi` for per-GPU total + *free* memory. Works under WSL (the wrapper at
/// `/usr/lib/wsl/lib/nvidia-smi` is on PATH). No link-time CUDA dependency.
#[cfg(not(target_os = "macos"))]
fn nvidia_smi() -> Result<Vec<Accelerator>, String> {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,memory.total,memory.free",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .map_err(|e| format!("nvidia-smi spawn failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("nvidia-smi exited with {:?}", out.status.code()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut gpus = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if cols.len() < 4 {
            continue;
        }
        gpus.push(Accelerator {
            kind: AccelKind::Cuda,
            index: cols[0].parse().unwrap_or(0),
            name: cols[1].to_string(),
            total_mb: cols[2].parse().unwrap_or(0),
            free_mb: cols[3].parse().unwrap_or(0),
        });
    }
    if gpus.is_empty() {
        return Err("nvidia-smi reported no devices".to_string());
    }
    Ok(gpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_yields_valid_report_without_panicking() {
        let r = detect();
        assert!(!r.platform.is_empty());
        assert!(!r.arch.is_empty());
        assert!(r.host_ram_total_mb > 0, "should detect physical RAM");
        if let Some(g) = r.best_gpu() {
            assert!(!g.name.is_empty());
        }
    }

    // 6000 = MARKER_FLOOR_MB in placement.rs (hardcoded here to avoid coupling the tests to
    // the planner module).
    const FLOOR_MB: u64 = 6000;

    #[test]
    fn unified_budget_24gb_mac_transient_low_free_stays_on_metal() {
        // Real Mac scenario: 24GB total, ~11.2GB free at launch.
        // max(11256-6144=5112, 24576-(6144+6800)=11632) = 11632 → Metal fits.
        let budget = apple_unified_gpu_budget_mb(24576, 11256);
        assert_eq!(budget, 11632);
        assert!(budget > FLOOR_MB);
    }

    #[test]
    fn unified_budget_24gb_mac_plenty_free_uses_instantaneous_term() {
        // max(20000-6144=13856, 11632) = 13856 → instantaneous term wins.
        assert_eq!(apple_unified_gpu_budget_mb(24576, 20000), 13856);
    }

    #[test]
    fn unified_budget_16gb_tight_machine_stays_cpu() {
        // max(8000-6144=1856, 16384-12944=3440) = 3440 → below the floor, CPU.
        let budget = apple_unified_gpu_budget_mb(16384, 8000);
        assert_eq!(budget, 3440);
        assert!(budget < FLOOR_MB);
    }

    #[test]
    fn unified_budget_8gb_machine_is_zero() {
        // Both terms saturate to 0 → CPU.
        assert_eq!(apple_unified_gpu_budget_mb(8192, 5000), 0);
    }
}
