//! Runtime detection of accelerators + free memory. Degrade-safe: every failure path yields
//! a valid report (CPU-only at worst); this function never panics.

use crate::report::{AccelKind, Accelerator, AcceleratorReport};

const BYTES_PER_MIB: u64 = 1024 * 1024;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const APPLE_OS_RESERVE_MB: u64 = 6144;

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
    let free_mb = ram_free_mb.saturating_sub(APPLE_OS_RESERVE_MB);
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
}
