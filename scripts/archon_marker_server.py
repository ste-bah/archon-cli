#!/usr/bin/env python3
"""Archon Marker server — persistent HTTP wrapper around the SAME conversion the subprocess
sidecar runs, with the surya models loaded ONCE at startup and kept resident.

Why: `archon docs ingest` on a bulk corpus spawns a fresh Python per PDF via the sidecar, so
every document pays a ~6 GB model reload with the GPU idle. This server loads the models once
and serves every document warm; archon selects it by setting `marker_url` in
`.archon/policy.toml` (MarkerSource::Http).

Contract (matched by `crates/archon-docs/src/marker_source.rs`):
    POST /convert   {"pdf_id": "<opaque canonical-path SHA-256>", "device": "cuda", "page_range": "S-E"?}
        → 200, the exact normalized block-tree JSON the sidecar prints to stdout
          (same shared core in archon_marker_core.py, same json.dumps(ensure_ascii=False))
        → 400 for an unknown `pdf_id` or invalid request/page range,
          and 500 with fixed safe messages on conversion failure; detailed failures
          are written only to local server logs
    GET /health     → 200 {"status": "ok", "device": "<startup device>", "models_loaded": true}

`--pdf-root` is required. The server recursively freezes canonical regular PDFs beneath it at
startup, indexed by `sha256(str(canonical_path).encode("utf-8")).hexdigest()`. `/convert` accepts
only those opaque IDs, never a path or PDF bytes. Catalogue entries support nested files and
duplicate basenames because the canonical full pathname is hashed. The catalogue is static for the
process lifetime: restart after corpus additions, moves, deletions, or replacements; post-start
local corpus mutation is outside remote request control. The server binds to loopback by default;
it has no authentication. Supplying `--allow-non-loopback` explicitly accepts exposure for a
non-loopback host, but does not change startup catalogue construction.

The request's `device` is ADVISORY: the models live on the device resolved at startup; a
mismatching request device is logged and ignored (restart the server to change device).
Server and archon run on the same host — the server reads only startup-catalogued PDFs from the
local filesystem, so no PDF bytes cross the wire.

Usage:
    /home/you/.venv-marker/bin/python3.11 scripts/archon_marker_server.py \
        --device cuda --pdf-root /home/you/corpus --host 127.0.0.1 --port 8010
"""

import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import sys
import threading
import time
from types import MappingProxyType

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from archon_marker_core import resolve_device, run_marker  # noqa: E402


def log(msg: str) -> None:
    sys.stderr.write(f"[archon-marker-server] {time.strftime('%Y-%m-%dT%H:%M:%S')} {msg}\n")
    sys.stderr.flush()


def canonical_pdf_root(path: str) -> Path:
    """Resolve and validate the directory containing PDFs this server may access."""
    try:
        root = Path(path).expanduser().resolve(strict=True)
    except OSError:
        raise ValueError("pdf root must be an existing directory") from None
    if not root.is_dir():
        raise ValueError("pdf root must be an existing directory")
    return root


def pdf_id_for_path(canonical_path: Path) -> str:
    """Return the deterministic opaque ID for a canonical UTF-8 PDF path."""
    try:
        path_text = str(canonical_path)
        path_bytes = path_text.encode("utf-8")
    except UnicodeError:
        raise ValueError("canonical pdf path is not valid UTF-8") from None
    return hashlib.sha256(path_bytes).hexdigest()


def build_pdf_catalogue(pdf_root: Path) -> MappingProxyType:
    """Freeze the canonical regular PDFs within ``pdf_root`` under opaque IDs."""
    catalogue = {}
    for candidate in pdf_root.rglob("*"):
        try:
            canonical = candidate.resolve(strict=True)
            canonical.relative_to(pdf_root)
            if canonical.suffix.lower() != ".pdf" or not canonical.is_file():
                continue
            pdf_id = pdf_id_for_path(canonical)
        except (OSError, UnicodeError, ValueError):
            continue
        existing = catalogue.get(pdf_id)
        if existing is not None:
            if existing != canonical:
                raise ValueError("PDF catalogue ID collision")
            continue
        catalogue[pdf_id] = canonical
    return MappingProxyType(catalogue)


def validate_bind_host(host: str, allow_non_loopback: bool) -> None:
    """Require explicit opt-in before exposing this unauthenticated server externally."""
    if host.lower() == "localhost":
        return
    try:
        is_loopback = ipaddress.ip_address(host).is_loopback
    except ValueError:
        is_loopback = False
    if not is_loopback and not allow_non_loopback:
        raise ValueError("non-loopback --host requires --allow-non-loopback")


# Marker pages are rendered into memory; accepting more than 1,000 pages in one
# request defeats the server's bounded, document-oriented conversion contract.
PAGE_RANGE_MAX_PAGES = 1000


def parse_page_range(spec: "str | None") -> "list[int] | None":
    """'S-E' (0-indexed, inclusive) → list of pages, exactly like the sidecar's --page-range.

    Rejects a reversed/empty range ('3-1') with ValueError so it can't reach Marker as an empty
    page list (which would silently convert nothing).
    """
    if not spec:
        return None
    try:
        start_text, end_text = spec.split("-", 1)
        if not start_text.isdecimal() or not end_text.isdecimal():
            raise ValueError
        start, end = int(start_text), int(end_text)
    except (AttributeError, ValueError):
        raise ValueError("invalid page_range") from None
    if end < start or end - start + 1 > PAGE_RANGE_MAX_PAGES:
        raise ValueError("invalid page_range")
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
    parser.add_argument(
        "--pdf-root", required=True, help="directory containing PDFs the server may read"
    )
    parser.add_argument(
        "--allow-non-loopback",
        action="store_true",
        help="allow an explicit non-loopback --host; does not add authentication",
    )
    args = parser.parse_args(argv)

    try:
        pdf_root = canonical_pdf_root(args.pdf_root)
        pdf_catalogue = build_pdf_catalogue(pdf_root)
        validate_bind_host(args.host, args.allow_non_loopback)
    except ValueError as exc:
        parser.error(str(exc))

    device = resolve_device(args.device)
    os.environ["TORCH_DEVICE"] = device  # set BEFORE importing marker so models land right
    log(f"loading surya models once on device={device} …")
    t0 = time.time()
    from marker.models import create_model_dict

    models = create_model_dict()
    log(f"models loaded in {time.time() - t0:.1f}s; resident for the process lifetime")

    app = build_app(device, models, pdf_catalogue)

    import uvicorn

    uvicorn.run(app, host=args.host, port=args.port, log_level="warning")
    return 0


def _convert_request_model():
    """Create the strict request model lazily for either supported Pydantic API."""
    import pydantic

    BaseModel = pydantic.BaseModel
    StrictStr = pydantic.StrictStr
    class ConvertRequest(BaseModel):
        pdf_id: StrictStr
        device: StrictStr | None = None
        page_range: StrictStr | None = None

        if hasattr(pydantic, "ConfigDict"):
            model_config = pydantic.ConfigDict(extra="forbid")
        else:

            class Config:
                extra = "forbid"

    return ConvertRequest


def _register_invalid_request_handler(app, request_validation_error, json_response) -> None:
    """Return a fixed response without exposing Pydantic validation detail."""
    @app.exception_handler(request_validation_error)
    async def invalid_request(_request, _exc):
        return json_response(status_code=400, content={"error": "invalid request"})


def _convert_with_oom_ladder(pdf_path, context, page_range):
    """Run Marker, preserving the shared GPU-to-CPU fallback and safe failures."""
    device = context["device"]
    try:
        return run_marker(str(pdf_path), device, page_range, artifact_dict=context["models"])
    except Exception as exc:  # noqa: BLE001 — classify OOM; log details locally
        if device == "cpu" or not is_cuda_oom(exc):
            empty_cuda_cache()
            log(f"convert FAILED for {pdf_path}: {exc}")
            return context["json_response"](status_code=500, content={"error": "conversion failed"})
        log(f"CUDA OOM on {pdf_path}; clearing cache and retrying on CPU: {exc}")
        empty_cuda_cache()
        try:
            return run_marker(str(pdf_path), "cpu", page_range, artifact_dict=None)
        except Exception as cpu_exc:  # noqa: BLE001 — CPU also failed → hard error
            empty_cuda_cache()
            log(f"convert FAILED (GPU OOM + CPU) for {pdf_path}: {cpu_exc}")
            return context["json_response"](status_code=500, content={"error": "conversion failed"})
        finally:
            os.environ["TORCH_DEVICE"] = device


def _convert_response(req, context):
    """Validate one catalogue-backed request and serialize its conversion result."""
    device = context["device"]
    if req.device and req.device not in ("auto", device):
        log(f"request device={req.device!r} ignored; models resident on {device!r}")
    pdf_path = context["pdf_catalogue"].get(req.pdf_id)
    if pdf_path is None:
        return context["json_response"](status_code=400, content={"error": "invalid pdf_id"})
    try:
        page_range = parse_page_range(req.page_range)
    except ValueError:
        return context["json_response"](status_code=400, content={"error": "invalid page_range"})
    log(f"convert pdf={pdf_path} page_range={req.page_range or 'all'}")
    started = time.time()
    with context["convert_lock"]:
        tree = _convert_with_oom_ladder(pdf_path, context, page_range)
        empty_cuda_cache()
    if not isinstance(tree, dict):
        return tree
    log(f"convert done in {time.time() - started:.1f}s pdf={pdf_path}")
    return context["response"](content=json.dumps(tree, ensure_ascii=False), media_type="application/json")


def build_app(device: str, models: "dict", pdf_catalogue: MappingProxyType):
    """Build the FastAPI app around resident models and a frozen PDF catalogue."""
    from fastapi import FastAPI
    from fastapi.exceptions import RequestValidationError
    from fastapi.responses import JSONResponse, Response

    app = FastAPI(title="archon-marker-server")
    _register_invalid_request_handler(app, RequestValidationError, JSONResponse)
    convert_lock = threading.Lock()
    context = {
        "device": device,
        "models": models,
        "pdf_catalogue": pdf_catalogue,
        "convert_lock": convert_lock,
        "json_response": JSONResponse,
        "response": Response,
    }
    ConvertRequest = _convert_request_model()

    @app.get("/health")
    def health():
        return {"status": "ok", "device": device, "models_loaded": True}

    @app.post("/convert")
    def convert(req: ConvertRequest):
        return _convert_response(req, context)

    return app


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
