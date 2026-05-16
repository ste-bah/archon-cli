# World Model MLX Metal JEPA Validation

Validation status: pending real Apple Silicon hardware.

## Current State

The MLX Metal JEPA path is implemented behind the `mlx-metal` feature and the
macOS Apple Silicon target gate:

```text
feature = "mlx-metal"
target_os = "macos"
target_arch = "aarch64"
```

On non-Apple-Silicon targets, the implementation compiles through explicit
fail-closed stubs. The Linux/WSL validation command below has passed:

```bash
cargo check -p archon-world-model --features mlx-metal --lib
```

That check proves the unsupported-target path builds. It does not validate MLX
Metal execution.

## Required Apple Silicon Validation

Run on a real Apple Silicon Mac:

```bash
cargo check -p archon-world-model --features mlx-metal --lib
cargo test -p archon-world-model --features mlx-metal --lib jepa -- --test-threads=2
```

The hardware run must verify the MLX equivalents of the CUDA evidence:

- `selected_backend = Metal`
- `framework = "mlx-rs"`
- real device name present
- `feature_compiled = true`
- `tensor_self_test_passed = true`
- `hardware_validation_captured_at = Some(...)`
- `validation_example_count >= min_metal_validation_examples`
- all required native JEPA stages set to `true`
- `native_runtime_prediction = Some(true)`
- `host_fallback_count = 0`
- CPU-vs-Metal frozen-forward parity meets `backend_parity_cosine_floor`

## Evidence To Record

When Apple Silicon validation is run, update this file with:

- Mac model
- chip type
- macOS version
- MLX and `mlx-rs` versions
- command outputs
- test counts
- validation date
- commit SHA
- candidate id
- execution report path or manifest path

Until a real Apple Silicon candidate manifest contains this execution evidence,
MLX Metal remains experimental and must not be described as fully hardware
validated.
