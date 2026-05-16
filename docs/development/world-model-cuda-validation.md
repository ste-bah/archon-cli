# World Model CUDA JEPA Validation

Validation date: 2026-05-16

Validation source: `archon/jepa-world-model-m2` working tree after the PRD-006F
accelerator finalization changes.

## Hardware

- GPU: NVIDIA GeForce RTX 4090
- Driver version reported by WSL: 591.74
- NVIDIA-SMI version: 590.52.01
- Driver-supported CUDA version reported by `nvidia-smi`: 13.1
- CUDA toolkit used for passing build/runtime validation: `/usr/local/cuda-13.1`
- `nvcc`: CUDA compilation tools 13.1, V13.1.115
- CUDA 13.2 toolkit note: `/usr/local/cuda-13.2/bin/nvcc` is present and was
  checked after `source ~/.profile`, but this driver advertises CUDA 13.1 and
  rejects Candle's CUDA 13.2-generated embedded PTX at runtime with
  `CUDA_ERROR_UNSUPPORTED_PTX_VERSION`. Use 13.1 here until the WSL driver
  supports the 13.2 PTX level.
- OS: Ubuntu 24.04.4 LTS on WSL2
- Kernel: `5.15.167.4-microsoft-standard-WSL2`

## Environment

The requested CUDA 13.2 path was checked with:

```bash
source ~/.profile || true
export CUDA_HOME=/usr/local/cuda-13.2
export CUDA_PATH=/usr/local/cuda-13.2
export CUDA_ROOT=/usr/local/cuda-13.2
export PATH=/usr/local/cuda-13.2/bin:$PATH
```

The passing validation used the driver-compatible toolkit:

```bash
source ~/.profile || true
export CUDA_HOME=/usr/local/cuda-13.1
export CUDA_PATH=/usr/local/cuda-13.1
export CUDA_ROOT=/usr/local/cuda-13.1
export NVCC=/usr/local/cuda-13.1/bin/nvcc
export PATH=/usr/local/cuda-13.1/bin:$PATH
export LD_LIBRARY_PATH=/usr/local/cuda-13.1/targets/x86_64-linux/lib:/usr/lib/wsl/lib:${LD_LIBRARY_PATH:-}
export CUDA_COMPUTE_CAP=89
```

## Commands

```bash
cargo check -p archon-world-model --features cuda --lib
cargo test -p archon-world-model --features cuda --lib jepa -- --test-threads=2
cargo test -p archon-world-model --features cuda --lib jepa_cuda -- --ignored --nocapture --test-threads=1
cargo test -p archon-world-model --features cuda --lib -- --test-threads=2
cargo test --bin archon --features cuda world_model::tests -- --test-threads=2
cargo test --bin archon --features cuda world_model::tests::predict_next_uses_active_jepa_cuda_model -- --ignored --nocapture --test-threads=1
```

## Results

- `cargo check -p archon-world-model --features cuda --lib`: passed.
- `cargo test -p archon-world-model --features cuda --lib jepa -- --test-threads=2`: 28 passed, 0 failed, 7 ignored hardware tests.
- `cargo test -p archon-world-model --features cuda --lib jepa_cuda -- --ignored --nocapture --test-threads=1`: 7 passed, 0 failed.
- `cargo test -p archon-world-model --features cuda --lib -- --test-threads=2`: 124 passed, 0 failed, 7 ignored hardware tests.
- `cargo test --bin archon --features cuda world_model::tests -- --test-threads=2`: 25 passed, 0 failed, 1 ignored hardware test.
- `cargo test --bin archon --features cuda world_model::tests::predict_next_uses_active_jepa_cuda_model -- --ignored --nocapture --test-threads=1`: 1 passed, 0 failed.

The CUDA JEPA tests include:

- `cuda_jepa_training_writes_native_execution_proof_when_available`
- `cuda_jepa_training_can_meet_hardware_validation_floor_when_available`
- `cuda_metadata_without_native_execution_proof_is_rejected`
- `jepa_cuda_probe_passes_tensor_self_test`
- `jepa_cuda_trains_encoder_predictor_aux_transition_native`
- `jepa_cuda_runtime_prediction_uses_cuda`
- `jepa_cuda_no_host_fallback_for_required_stages`
- `jepa_cuda_candidate_metadata_records_cuda`
- `jepa_cuda_promotion_requires_backend_proof`
- `jepa_cuda_parity_with_cpu_fixture`
- `predict_next_uses_active_jepa_cuda_model`

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

The promotion gate now rejects CUDA/Metal candidates unless
`native_runtime_prediction = Some(true)` is present alongside the native
training-stage proof.

The test candidate id is UUID-based and lives in the test process; it is not a
stable repository artifact. Production promotion still requires the real
candidate manifest under the user's world-model store to carry the same report.
