#!/usr/bin/env python3
"""Archon Marker core — the shared "PDF path (+ optional page range) → normalized block-tree
dict" conversion used by BOTH transports:

- `archon_marker_sidecar.py` (per-doc subprocess; models load fresh each run), and
- `archon_marker_server.py` (persistent HTTP server; models load once and stay resident).

Keeping the conversion + normalization in this ONE place guarantees the two transports emit
byte-identical block trees for the same PDF — if they drifted, archon's chunks_root provenance
would diverge by transport. Do not duplicate this logic in the callers.
"""

import os


def resolve_device(explicit: str | None) -> str:
    """cuda → mps → cpu. An explicit device / `TORCH_DEVICE` overrides auto-detection."""
    chosen = explicit or os.environ.get("TORCH_DEVICE")
    if chosen and chosen != "auto":
        return chosen
    try:
        import torch  # noqa: WPS433 (local import: keep torch-free callers torch-free)

        if torch.cuda.is_available():
            return "cuda"
        if getattr(torch.backends, "mps", None) is not None and torch.backends.mps.is_available():
            return "mps"
    except Exception:  # torch absent or broken → CPU is always valid
        pass
    return "cpu"


def run_marker(
    pdf_path: str,
    device: str,
    page_range: "list[int] | None" = None,
    artifact_dict: "dict | None" = None,
) -> dict:
    """Run Marker with its JSON renderer and return the block tree as a dict.

    Marker's JSON renderer already emits `{block_type, id, html, bbox/polygon, children}`,
    which is exactly the Rust parser's contract. `TORCH_DEVICE` is set before importing
    Marker so its models load on the resolved device.

    `page_range` (0-indexed original-PDF page numbers) restricts the run to those pages; Marker
    keeps ABSOLUTE page ids (`/page/N/`), so a caller can process a big PDF in page-range chunks
    that each fit VRAM and concatenate the block streams without re-offsetting. `None` = whole doc.

    `artifact_dict` lets a persistent caller (the HTTP server) pass pre-loaded surya models so
    they are loaded ONCE per process instead of once per document; `None` (the subprocess
    sidecar's case) loads them fresh. The conversion itself is identical either way.

    [CONFIRM on the Mac] against the installed Marker version: the converter/renderer import
    paths and whether boxes come back as `bbox` (used here) or `polygon` (map to a bbox).
    """
    os.environ["TORCH_DEVICE"] = device
    import json

    from marker.converters.pdf import PdfConverter
    from marker.models import create_model_dict

    if artifact_dict is None:
        artifact_dict = create_model_dict()
    config = {"page_range": page_range} if page_range is not None else {}
    converter = PdfConverter(
        artifact_dict=artifact_dict,
        config=config,
        renderer="marker.renderers.json.JSONRenderer",
    )
    rendered = converter(pdf_path)
    # `rendered` is a pydantic model with `.children`; round-trip via JSON for a plain dict.
    if hasattr(rendered, "model_dump"):
        return rendered.model_dump(mode="json")
    if hasattr(rendered, "dict"):
        return rendered.dict()
    return json.loads(rendered.json())
