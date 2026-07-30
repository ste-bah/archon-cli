# Hardware & VRAM guide — device-adaptive PDF ingestion

Archon's PDF ingestion is **device-adaptive**: it detects each host's *free* VRAM at runtime and
places work accordingly (NVIDIA CUDA → Apple Metal/MPS → CPU). This guide lists what runs on what,
so you can pick a working configuration for your machine.

There are two GPU consumers, and they run **sequentially**, never at the same instant:

1. **Marker** — PDF layout + real per-block bounding boxes (the provenance-critical path).
2. **VLM** *(optional)* — a natural-language *description* of each embedded figure. Separate from
   OCR, which always extracts text *inside* images on CPU.

Marker runs as a subprocess that **exits and frees its VRAM before** the image-enrichment/VLM step
begins (`ingest_pdf.rs`: chunks + `chunks_root` are persisted, then `enrich_pdf_images` runs). So a
single document never needs Marker-VRAM + VLM-VRAM at the same moment — but see
[Residency](#residency-the-real-constraint) for the cross-document caveat.

---

## Measured footprints

All figures measured with `ollama ps` / `mem_probe.py` (marker-pdf 1.10.2 / surya 0.17.1). GPU
resident size **includes the KV cache**, which for VLMs is dominated by the context window.

| Component | VRAM | Notes |
|---|---|---|
| **Marker** | **~6.0 GB floor → 10.0 GB cap** (page-scaled `min(6000 + 30·pages, 10240)` MiB) | Bounded/saturating, *not* batch-driven. Big docs on small cards are auto-split into page-range GPU chunks; per-doc OOM → CPU fallback. |
| **Embedding** (fastembed) | CPU-only this round (~0.5 GB RAM) | Not a GPU consumer. |
| **VLM `qwen2.5vl:7b`** | 22 GB @32K ctx · 15 GB @8K · **14 GB @4K** | Recommended model. Big vision tower — heavier than its ~6 GB weights suggest. |
| **VLM `qwen2.5vl:3b`** | 15 GB @32K · **11 GB @4K** | Smaller, but still >8 GB even at 4K context. |
| **VLM `moondream`** | **1.3 GB** @2K | Tiny/fits anything, but low & unreliable quality (empty outputs observed). Last resort. |
| **VLM `llama3.2-vision:11b`** | 16 GB @32K | ⚠ `mllama` arch — runs only on **older** Ollama; **dropped in current Ollama**. Avoid for portability. |

**Two levers shrink the VLM a lot:** lowering `num_ctx` (image description needs ~2–4K, not 32K)
and `kv_cache_type=q8_0` (halves the KV cache). The defaults are wasteful for this workload.

---

## What runs on what VRAM

| VRAM (free) | Marker | VLM (figure descriptions) |
|---|---|---|
| **None (CPU-only / CI)** | CPU (10–50× slower) | CPU or cloud |
| **8 GB** (e.g. RTX 5070 laptop) | ✅ GPU, page-range-chunked for big docs | ❌ No Qwen-VL size fits GPU (≥11 GB). → **VLM on CPU** (full quality, slower) or disable in the field |
| **12 GB** | ✅ GPU | `qwen2.5vl:3b`/`7b` @ low ctx, **sequential only**; or CPU |
| **16 GB** (RTX 4070 Ti S, 4060 Ti-16) | ✅ GPU | `qwen2.5vl:7b` @4K (14 GB) fits **one-at-a-time**; set short `keep_alive` |
| **24 GB** (Mac unified, RTX 3090/4090) | ✅ GPU / MPS | `qwen2.5vl:7b` fits; reduce ctx + short `keep_alive` so it doesn't overlap the next doc's Marker. Apple: budget ≈ total − ~6 GB OS |
| **32 GB+** (RTX 5090) | ✅ GPU, full | `qwen2.5vl:7b` at default ctx + Marker, comfortable headroom |

> The numbers assume Marker (≤10 GB) and one VLM run **do not need to coexist** — which is true
> within a document. The risk is a VLM model *lingering* resident into the next document's Marker
> (below).

---

## Residency: the real constraint

On constrained VRAM the slowdown isn't Marker-vs-VLM *within* a document — it's the VLM model
**staying resident across documents**. Ollama keeps a model loaded for `keep_alive` (default 5 min).
If a 14–22 GB vision model is still resident when the *next* document's Marker starts, Marker sees
less free VRAM and is forced to page-range-chunk (slow) or fall back to CPU.

Mitigations (recommended for ≤24 GB hosts):

- **Only load the VLM when a document actually has images.** Already the default behavior: the
  pipeline calls the VLM per *extracted image*, and Ollama lazy-loads — a **text-only document never
  spins up the VLM**, leaving all VRAM for Marker.
- **Unload the VLM promptly** — short/zero `keep_alive` so it releases VRAM right after a document's
  figures are described, before the next document's Marker.
- **Cap `num_ctx`** (~2–4K) and use `kv_cache_type=q8_0` to shrink the resident model.
- **8 GB hosts:** run the VLM on **CPU** (`OLLAMA_NUM_GPU=0`) so Marker owns the GPU outright.

> **Planned archon knobs** (not yet exposed): `[policy.docs.vlm.ollama] num_ctx`, `keep_alive`,
> `num_gpu`/`prefer_cpu`. Until then these are set via the Ollama server env / per-request options.

---

## Recommended configs (this fleet)

- **5090 (32 GB)** — `provider=ollama`, `model=qwen2.5vl:7b`, default context. Ingest primary.
- **Mac (24 GB unified, MPS)** — `qwen2.5vl:7b`; plan to cap `num_ctx≈4096` + short `keep_alive`.
- **RTX 5070 laptop (8 GB) — LOCKED:** Marker on GPU, **VLM = `qwen2.5vl:7b` on CPU**
  (`OLLAMA_NUM_GPU=0` so Marker owns the GPU; same model as the other machines → consistent
  descriptions, just slower for the image pass). Text-only docs skip the VLM entirely, so this only
  costs time on image-bearing docs. (Field alternative: disable VLM, enrich later on the 5090.)

## Cross-platform notes

- **NVIDIA CUDA** (Linux/WSL): free VRAM via `nvidia-smi`; `expandable_segments` on to cut
  fragmentation OOM.
- **Apple Metal/MPS** (unified memory): budget ≈ total − ~6 GB OS reserve; watch memory pressure
  (soft, not a catchable OOM) and back off.
- **CPU**: always the safe fallback for both Marker and VLM; correct, just slow.

Model choice: **`qwen2.5vl`** is the portable default — it runs on both old and current Ollama.
Avoid `llama3.2-vision` (`mllama`) unless you pin an older Ollama.
