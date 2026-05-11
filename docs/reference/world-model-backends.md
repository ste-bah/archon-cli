# World Model Backends

The world model keeps the user-facing backend flag stable:

```toml
[learning.world_model.training]
backend = "auto" # auto | cpu | cuda | metal
allow_cpu_fallback = true
precision = "fp32"
```

## Support Matrix

| Backend | Framework | Feature | Status | Notes |
|---|---|---|---|---|
| `cpu` | Compact CPU path; Candle 0.10.2 forward path under feature | default / `candle` | Supported | Consumer-friendly default; `candle` enables Candle tensor execution. |
| `cuda` | Candle CUDA path | `cuda` | Feature-gated | Linux/WSL GPU training/inference path. Local CUDA-feature validation passes with `/usr/local/cuda-13.2` after driver/toolkit compatibility is corrected. |
| `metal` | `mlx-rs = 0.25.3` | `mlx-metal` | Experimental | Apple Silicon MLX training/inference path. Native execution remains experimental until a maintainer records real hardware validation. |
| `auto` | selector | default | Supported | Prefers available accelerator, otherwise CPU when fallback is allowed. |

## Availability Probe

Every accelerator backend must pass `probe()` before Archon selects it or trains on it. The probe creates the device, runs a tiny tensor operation, synchronizes by reading the result back to the host, and records a structured reason if it fails. Device creation alone is not an availability signal.

This catches CUDA runtime issues such as toolkit/driver minor-version mismatches before foreground work depends on the accelerator. For CUDA, the practical fixes are to upgrade the driver, install a toolkit matching the driver-supported CUDA version, or compile for a concrete target architecture such as `sm_89` where the lower-level toolchain supports it. NVIDIA documents the PTX and `nvcc -arch=sm_xx` caveats in its [CUDA minor-version compatibility guide](https://docs.nvidia.com/deploy/cuda-compatibility/minor-version-compatibility.html).

## Failure Posture

Backend failures must not block foreground Archon work. Accelerator probe failures emit backend-specific fallback reasons and can retry on CPU when `allow_cpu_fallback = true`.

The WSL implementation validates CPU behavior, backend selection, checkpoint
metadata, Candle safetensors roundtrips, MLX array checkpoint artifacts, and
bridge/parity test metadata. CUDA-feature library validation passes locally
on WSL. Apple Silicon execution must be validated on matching hardware.

Promotion compares candidates from the same backend by default. Cross-backend parity is a forward-pass check: train on Candle CPU, bridge weights, run Candle CPU and MLX Metal forward passes on the same heldout data, and require output cosine similarity of at least `0.95` using `fp32` parity.

## Checkpoint Formats

| Backend | Format |
|---|---|
| CPU | Candle safetensors |
| CUDA | Candle safetensors |
| Metal | MLX array checkpoint |
