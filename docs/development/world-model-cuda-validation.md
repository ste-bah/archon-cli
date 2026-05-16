# World Model CUDA JEPA Validation

Validation date: 2026-05-16

Validation commit: `66162926c4d7c3fba9064175dcae21532163f71c`

## Hardware

- GPU: NVIDIA GeForce RTX 4090
- Driver version reported by WSL: 591.74
- NVIDIA-SMI version: 590.52.01
- Driver-supported CUDA version reported by `nvidia-smi`: 13.1
- CUDA toolkit used for build: `/usr/local/cuda-13.2`
- `nvcc`: CUDA compilation tools 13.2, V13.2.51
- OS: Ubuntu 24.04.4 LTS on WSL2
- Kernel: `5.15.167.4-microsoft-standard-WSL2`

## Environment

```bash
source ~/.profile || true
export CUDA_HOME=/usr/local/cuda-13.2
export CUDA_PATH=/usr/local/cuda-13.2
export CUDA_ROOT=/usr/local/cuda-13.2
export PATH=/usr/local/cuda-13.2/bin:$PATH
```

## Commands

```bash
cargo check -p archon-world-model --features cuda --lib
cargo test -p archon-world-model --features cuda --lib jepa -- --test-threads=2
cargo test -p archon-world-model --features cuda --lib -- --test-threads=2
```

## Results

- `cargo check -p archon-world-model --features cuda --lib`: passed.
- `cargo test -p archon-world-model --features cuda --lib jepa -- --test-threads=2`: 27 passed, 0 failed.
- `cargo test -p archon-world-model --features cuda --lib -- --test-threads=2`: 123 passed, 0 failed.

The CUDA JEPA tests include:

- `cuda_jepa_training_writes_native_execution_proof_when_available`
- `cuda_jepa_training_can_meet_hardware_validation_floor_when_available`
- `cuda_metadata_without_native_execution_proof_is_rejected`

The 512-example hardware validation floor is covered by
`cuda_jepa_training_can_meet_hardware_validation_floor_when_available`. That
test trains a CUDA-labelled JEPA candidate on a deterministic 520-row fixture,
asserts `validation_example_count >= 512`, asserts `host_fallback_count == 0`,
and requires `jepa_backend_promotion_gate(..., 512, 512)` to pass.

## Candidate Execution Report

The promotion gate is the candidate manifest's `JepaBackendExecutionReport`, not
this markdown file. The hardware validation fixture creates an in-test
candidate manifest and verifies that its execution report has:

- `selected_backend = Cuda`
- `framework = "candle"`
- `feature_compiled = true`
- `tensor_self_test_passed = true`
- `hardware_validation_captured_at = Some(...)`
- `validation_example_count >= 512`
- all required native JEPA stages set to `true`
- `native_runtime_prediction = Some(true)`
- `host_fallback_count = 0`

The test candidate id is UUID-based and lives in the test process; it is not a
stable repository artifact. Production promotion still requires the real
candidate manifest under the user's world-model store to carry the same report.
