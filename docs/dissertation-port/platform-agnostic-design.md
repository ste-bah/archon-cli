# Platform-Agnostic Design

The central property of the ingestion port: **only the Marker Python sidecar is
device-sensitive; the entire Rust core is portable, and cross-platform output is
byte-identical.** The same pipeline runs on Apple Metal/MPS, NVIDIA CUDA
(including under WSL), and CPU with no code change, and a document ingested on
one device produces the same chunks — and therefore the same integrity hash — as
on any other.

---

## 1. One Marker core, every device

`scripts/archon_marker_sidecar.py` (per-doc subprocess) and
`scripts/archon_marker_server.py` (persistent HTTP server) both call the **same**
`scripts/archon_marker_core.py`:

- `resolve_device` auto-detects `cuda → mps → cpu`, overridable via `--device`
  or the `TORCH_DEVICE` env var.
- `run_marker` runs Marker's `PdfConverter` with the `JSONRenderer` and
  `json.dumps(..., ensure_ascii=False)`.

The identical script therefore runs on a Mac (Metal/MPS), an NVIDIA host or WSL
box (CUDA), or CPU with no change — only the resolved device differs. Keeping
conversion + normalization in one shared core is what guarantees the two
transports emit **byte-identical** block trees for the same PDF; if they drifted,
the `chunks_root` provenance would diverge by transport. A checked-in golden
fixture (`archon-ingest-ext/tests/fixtures/marker_selftest.json`, produced by the
sidecar's torch-free `--selftest`) pins the parser contract.

Transport is orthogonal to device: subprocess vs HTTP vs pre-extracted JSON is
chosen on the Rust side (`MarkerSource`) independently of which device Marker uses.

### HTTP security boundary

The persistent server requires `--pdf-root`; every requested absolute PDF path is
canonicalized, must end in `.pdf`, and must remain beneath that root. It binds to
loopback by default and has no authentication. A non-loopback `--host` requires
explicit `--allow-non-loopback` risk acceptance, which never weakens root
containment. The server returns fixed safe 400/500 messages while retaining detailed
conversion failures in local logs only. These transport controls do not change the
shared conversion output or device placement described above.

---

## 2. `archon-accel` — uniform detection + placement across CUDA / Metal / CPU

`archon-accel` separates a runtime **detect** probe from a pure **placement**
planner.

- **`AccelKind::{Cuda, Metal, Cpu}`** with a `sidecar_device()` mapping
  (`cuda` / `mps` / `cpu`).
- **`detect()`** is `cfg`-split but yields one uniform `AcceleratorReport`:
  - non-macOS → parse `nvidia-smi` (works under WSL via the on-PATH wrapper; no
    link-time CUDA dependency),
  - Apple Silicon → synthesize a single `Metal` accelerator over the unified pool,
  - Intel Mac → CPU.
  It never panics — CPU-only is the worst case.
- The planner (`plan_placement`, `marker_ingest_plan`) is **pure and
  deterministic**, so it is fully unit-testable on CPU-only CI with no GPU.

### Free VRAM, not card size

The load-bearing selector is *free* memory: `AcceleratorReport::best_gpu()` picks
the GPU with the most free VRAM. A 32 GB card with ~139 MB free under co-tenancy
correctly routes to CPU. This is why the pipeline behaves well when a game or
another model already holds the GPU.

### Page-scaled Marker footprint + page-range chunking

Marker's VRAM is driven by surya's whole-document OCR pass, so it rises from a
~6 GB model floor and **saturates** (it is *not* batch-driven):

```
marker_footprint_mb(pages) = min(6000 + 30·pages, 10240)   MiB
```

fit to three measured points (13 pp → 5956, 129 pp → 9424, 578 pp → 8470 MiB;
marker-pdf 1.10.2 / surya 0.17.1 on RTX 5070 + Apple MPS). Its inverse
`marker_chunk_pages(usable)` gives the largest page-range that fits free VRAM, so
a big document is split into contiguous `--page-range` chunks that each fit a
small card. Marker emits **absolute** page ids, so the chunk block streams
concatenate in order with no re-offset. Batch size is VRAM-inert, so page-range
is the only real lever.

### GPU → CPU OOM ladder

Every `MarkerChunk.attempts` is `[GPU, CPU]`. `run_chunk` / `run_sidecar` walk the
ladder, advancing on a torch-OOM; a GPU placement always sets
`oom_fallback_to_cpu: true` — the real correctness guarantee, since the footprint
estimate is unverified. The persistent server mirrors the ladder in-process
(CUDA-OOM → `empty_cache()` → retry on CPU, with `empty_cache()` after every
attempt to keep fragmentation from cascading across a large run).

### Apple unified-memory budget

`apple_unified_gpu_budget_mb(total, free)` takes the larger of (a) instantaneous
free minus a 6144 MB OS reserve and (b) total minus OS + a **6800 MB
coresident-VLM reserve** (measured for `qwen2.5vl:7b` on Metal) — but the
free-independent term is **bounded to 1.5× instantaneous free**, so a 64 GB
machine with only 3 GB free reports 4608 MB (→ CPU), not a runaway ~51 GB
over-report. On real hardware this is what lets Marker run on the Mac's MPS when
memory is available and fall back cleanly when it is not.

### `--jobs auto` — VRAM-adaptive enrichment concurrency

`auto_image_workers(report)` budgets VLM-enrichment worker slots from free VRAM
after reserving the VLM model weights (`VLM_MODEL_RESERVE_MB = 6500`) and a
`VLM_HEADROOM_MB = 2048` margin, clamped to `1..=16` and capped at **2** on
unified memory. On an 8 GB card it correctly resolves to a single serial worker.

---

## 3. Byte-identical chunk parity across platforms

The token-aware chunker (`archon-ingest-ext/chunk.rs`) and the Marker parser
(`marker.rs`) are pure Rust with no device dependency. The token estimate counts
Unicode **code points / 4** (not bytes), so it matches the Python reference on
Greek and German text.

The parity contract is documented in
[`../ingestion/parity-divergences.md`](../ingestion/parity-divergences.md): the
chunk-budget constants (`TARGET_MIN` / `TARGET_MAX` / `HARD_MAX`) and the boundary
algorithm must match the Python reference byte-exact, with six intentional Rust
improvements registered as expected divergences. It is enforced by
`scripts/chunk_parity_check.py` and a golden test
(`chunk_parity_matches_python_reference`).

---

## 4. Verify-by-recompute, not device-dependent quantization

Integrity is anchored to **content**, not float-exact geometry. A GPU chunk that
OOMs and falls back to CPU can produce bounding boxes that differ by a few pixels
between devices. Rather than force bit-identical coordinates (device-specific
quantization), the design accepts that jitter and verifies by recomputing a
`chunks_root` hash over chunk content (`chunk_id`, raw/clean sha256, spatial
hash, cleaning version). The cross-device bbox-jitter study (max Δ ≈ 7.7 px,
p50 = 0) confirmed no quantization bucket reached a safe unify rate, so
verify-by-recompute is kept — and it makes the whole scheme device-agnostic by
construction.

---

## 5. Hang-hardening is device-agnostic too

Every wall-clock backstop is portable Rust with `kill_on_drop(true)`:

| Backstop | Env var | Default |
|---|---|---|
| GPU Marker convert (a hang → ladder falls to CPU) | `ARCHON_MARKER_GPU_TIMEOUT_SECS` | 900 s |
| CPU Marker convert (last rung, legitimately slow) | `ARCHON_MARKER_CPU_TIMEOUT_SECS` | 3600 s |
| One OCR subprocess (tesseract / RapidOCR / pdftotext) | `ARCHON_OCR_TIMEOUT_SECS` | 120 s |
| Whole-document `pdftoppm` render | `ARCHON_PDF_RENDER_TIMEOUT_SECS` | 1800 s |
| Per-image enrichment (OCR + VLM), skips one wedged image | `ARCHON_PDF_IMAGE_TIMEOUT_SECS` | 600 s |
| HTTP Marker convert | (constant) | 900 s |

A sidecar terminated by a signal with no exit code (e.g. an OS OOM-killer /
macOS jetsam SIGKILL, which never prints the torch-OOM signature) is treated like
an OOM and advances the ladder to CPU.

---

## 6. Validated fleet

| Machine | Device | Result |
|---|---|---|
| MacBook (24 GB unified) | Apple MPS | Marker on MPS, VLM (qwen2.5vl:7b) 6.8 GB on Metal, integrity clean |
| Laptop RTX 5070 (8 GB) | CUDA | `--jobs auto` = 1 serial; Marker on CUDA (~5.4 GB, fits) or CPU when the VLM holds the GPU; 3/3 clean |
| Workstation RTX 5090 (32 GB) | CUDA | full corpus flip — 124 docs, 123 with real bboxes, integrity 124/124 |

The only device-sensitive component is the Marker Python venv (torch build:
CUDA `cu128` for NVIDIA, the default MPS wheels for Apple). Everything else — the
Rust binary, chunking, integrity, placement logic — is the same artifact on every
platform.
