#!/usr/bin/env python3
"""Archon Marker server — persistent HTTP wrapper around the SAME conversion the subprocess
sidecar runs, with the surya models loaded ONCE at startup and kept resident.

Why: `archon docs ingest` on a bulk corpus spawns a fresh Python per PDF via the sidecar, so
every document pays a ~6 GB model reload with the GPU idle. This server loads the models once
and serves every document warm; archon selects it by setting `marker_url` in
`.archon/policy.toml` (MarkerSource::Http).

Contract (matched by `crates/archon-docs/src/marker_source.rs::fetch_json`):
    POST /convert   {"pdf_path": "<absolute path>", "device": "cuda", "page_range": "S-E"?}
        → 200, the exact normalized block-tree JSON the sidecar prints to stdout
          (same shared core in archon_marker_core.py, same json.dumps(ensure_ascii=False))
        → 400 on a missing/unreadable pdf_path, 500 with the error text on conversion failure
    GET /health     → 200 {"status": "ok", "device": "<startup device>", "models_loaded": true}

The request's `device` is ADVISORY: the models live on the device resolved at startup; a
mismatching request device is logged and ignored (restart the server to change device).
Server and archon run on the same host — the server reads `pdf_path` from the local
filesystem, so no PDF bytes cross the wire.

Usage:
    /home/dalton/.venv-marker/bin/python3.11 scripts/archon_marker_server.py \
        --device cuda --host 127.0.0.1 --port 8010
"""

import argparse
import json
import os
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from archon_marker_core import resolve_device, run_marker  # noqa: E402


def log(msg: str) -> None:
    sys.stderr.write(f"[archon-marker-server] {time.strftime('%Y-%m-%dT%H:%M:%S')} {msg}\n")
    sys.stderr.flush()


def parse_page_range(spec: "str | None") -> "list[int] | None":
    """'S-E' (0-indexed, inclusive) → list of pages, exactly like the sidecar's --page-range.

    Rejects a reversed/empty range ('3-1') with ValueError so it can't reach Marker as an empty
    page list (which would silently convert nothing).
    """
    if not spec:
        return None
    start, end = (int(x) for x in spec.split("-", 1))
    if end < start:
        raise ValueError(f"reversed/empty page_range: start {start} > end {end}")
    return list(range(start, end + 1))


def is_cuda_oom(exc: Exception) -> bool:
    """Classify a torch CUDA out-of-memory error the same way the subprocess sidecar does."""
    try:
        import torch

        if isinstance(exc, torch.cuda.OutOfMemoryError):
            return True
    except Exception:  # noqa: BLE001 — torch missing/broken → fall back to the message check
        pass
    return "out of memory" in str(exc).lower() or type(exc).__name__ == "OutOfMemoryError"


def empty_cuda_cache() -> None:
    """Release cached CUDA blocks so one OOM's fragmentation can't cascade to later docs."""
    try:
        import torch

        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except Exception:  # noqa: BLE001 — best-effort hygiene, never fatal
        pass


def main(argv: "list[str]") -> int:
    parser = argparse.ArgumentParser(description="Archon persistent Marker HTTP server.")
    parser.add_argument("--device", help="cuda|mps|cpu|auto (default/auto: cuda→mps→cpu)")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8010)
    args = parser.parse_args(argv)

    device = resolve_device(args.device)
    os.environ["TORCH_DEVICE"] = device  # set BEFORE importing marker so models land right
    log(f"loading surya models once on device={device} …")
    t0 = time.time()
    from marker.models import create_model_dict

    models = create_model_dict()
    log(f"models loaded in {time.time() - t0:.1f}s; resident for the process lifetime")

    app = build_app(device, models)

    import uvicorn

    uvicorn.run(app, host=args.host, port=args.port, log_level="warning")
    return 0


def build_app(device: str, models: "dict"):
    """Build the FastAPI app around a startup device + resident models. Extracted from `main`
    so the OOM ladder / endpoints are testable (TestClient + a fake `models`) without a real GPU.
    """
    from fastapi import FastAPI
    from fastapi.responses import JSONResponse, Response
    from pydantic import BaseModel

    app = FastAPI(title="archon-marker-server")
    # Marker conversion is not assumed thread-safe on shared models; archon ingests
    # sequentially anyway, so serialize conversions.
    convert_lock = threading.Lock()

    class ConvertRequest(BaseModel):
        pdf_path: str
        device: "str | None" = None
        page_range: "str | None" = None  # 'S-E', 0-indexed inclusive (sidecar --page-range)

    @app.get("/health")
    def health():
        return {"status": "ok", "device": device, "models_loaded": True}

    @app.post("/convert")
    def convert(req: ConvertRequest):
        if req.device and req.device not in ("auto", device):
            log(f"request device={req.device!r} ignored; models resident on {device!r}")
        if not os.path.isfile(req.pdf_path):
            return JSONResponse(
                status_code=400,
                content={"error": f"pdf_path not found: {req.pdf_path}"},
            )
        try:
            page_range = parse_page_range(req.page_range)
        except ValueError:
            return JSONResponse(
                status_code=400,
                content={"error": f"page_range must be 'START-END', got {req.page_range!r}"},
            )
        log(f"convert pdf={req.pdf_path} page_range={req.page_range or 'all'}")
        t = time.time()
        # OOM ladder mirroring the subprocess path (marker_source.rs run_chunk): try the resident
        # GPU device first; on a CUDA OOM, clear the cache and RETRY the whole-doc conversion on
        # CPU with the SAME shared core (byte-identical normalized JSON, just slower). Only a CPU
        # failure returns 500. empty_cache() runs after every attempt to prevent fragmentation
        # from cascading across the 126-doc run. All of it stays under the single convert_lock.
        with convert_lock:
            try:
                tree = run_marker(req.pdf_path, device, page_range, artifact_dict=models)
            except Exception as exc:  # noqa: BLE001 — classify OOM, otherwise surface
                if device != "cpu" and is_cuda_oom(exc):
                    log(f"CUDA OOM on {req.pdf_path}; clearing cache and retrying on CPU: {exc}")
                    empty_cuda_cache()
                    try:
                        # artifact_dict=None → load a FRESH CPU model dict (the resident dict is
                        # GPU-bound); the shared core still yields byte-identical normalized JSON.
                        tree = run_marker(req.pdf_path, "cpu", page_range, artifact_dict=None)
                    except Exception as cpu_exc:  # noqa: BLE001 — CPU also failed → hard error
                        empty_cuda_cache()
                        log(f"convert FAILED (GPU OOM + CPU) for {req.pdf_path}: {cpu_exc}")
                        return JSONResponse(status_code=500, content={"error": str(cpu_exc)})
                    finally:
                        # Restore device on the resident models after the CPU fallback.
                        os.environ["TORCH_DEVICE"] = device
                else:
                    empty_cuda_cache()
                    log(f"convert FAILED for {req.pdf_path}: {exc}")
                    return JSONResponse(status_code=500, content={"error": str(exc)})
            finally:
                empty_cuda_cache()
        log(f"convert done in {time.time() - t:.1f}s pdf={req.pdf_path}")
        # Same serialization call as the sidecar's stdout — byte-identical payloads.
        return Response(
            content=json.dumps(tree, ensure_ascii=False),
            media_type="application/json",
        )

    return app


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
