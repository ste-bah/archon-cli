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
| `cuda` | Candle CUDA path | `cuda` | Feature-gated | Linux/WSL GPU path. JEPA CUDA candidates must carry training-time hardware execution proof before they can promote. |
| `metal` | `mlx-rs = 0.25.3` | `mlx-metal` | Experimental | Apple Silicon MLX path. JEPA Metal candidates must carry real Apple Silicon execution proof before they can promote. |
| `auto` | selector | default | Supported | Prefers available accelerator, otherwise CPU when fallback is allowed. |

## Availability Probe

Every accelerator backend must pass `probe()` before Archon selects it or trains on it. The probe creates the device, runs a tiny tensor operation, synchronizes by reading the result back to the host, and records a structured reason if it fails. Device creation alone is not an availability signal.

This catches CUDA runtime issues such as toolkit/driver minor-version mismatches before foreground work depends on the accelerator. For CUDA, the practical fixes are to upgrade the driver, install a toolkit matching the driver-supported CUDA version, or compile for a concrete target architecture such as `sm_89` where the lower-level toolchain supports it. NVIDIA documents the PTX and `nvcc -arch=sm_xx` caveats in its [CUDA minor-version compatibility guide](https://docs.nvidia.com/deploy/cuda-compatibility/minor-version-compatibility.html).

## Failure Posture

Backend failures must not block foreground Archon work. Accelerator probe failures emit backend-specific fallback reasons and can retry on CPU when `allow_cpu_fallback = true`.

The non-accelerated implementation validates CPU behavior, backend selection,
checkpoint metadata, Candle safetensors roundtrips, MLX array checkpoint
artifacts, and bridge/parity test metadata. CUDA and Apple Silicon execution
must be validated on matching hardware and recorded in the JEPA-inspired candidate's
`JepaBackendExecutionReport`.

The current CUDA validation record is
[`docs/development/world-model-cuda-validation.md`](../development/world-model-cuda-validation.md).
The MLX Metal validation checklist is
[`docs/development/world-model-mlx-metal-validation.md`](../development/world-model-mlx-metal-validation.md);
Metal remains experimental until that checklist is filled from real Apple
Silicon hardware.

Promotion compares candidates from the same backend by default. Cross-backend
parity is a frozen-weights forward-pass check: train or build one deterministic
CPU checkpoint, bridge the same weights to CUDA or MLX Metal, run the same
fixture, and require output cosine similarity of at least
`jepa.backend_parity_cosine_floor` using `fp32` parity. It is not a full
cross-backend retrain parity gate.

## JEPA Acceleration Proof

A JEPA-inspired candidate may report `backend = "cuda"` or `backend = "metal"` only when
its candidate manifest includes a training-time `JepaBackendExecutionReport`
showing:

- the requested and selected backend;
- the framework and device name;
- feature compilation and tensor self-test success;
- native execution for encode, predictor fit, auxiliary-head fit, transition
  fit, and loss evaluation;
- `host_fallback_count = 0` for required stages;
- hardware validation timestamp, commit SHA, and enough validation examples for
  that backend.

If CUDA or Metal is requested but the native JEPA implementation is unavailable,
Archon either writes an explicitly CPU-labelled fallback candidate with a
fallback reason, or fails the training command when CPU fallback is disabled. It
must not write accelerator-flavoured metadata for CPU execution.

## Checkpoint Formats

| Backend | Format |
|---|---|
| CPU | Candle safetensors |
| CUDA | Candle safetensors |
| Metal | MLX array checkpoint |
