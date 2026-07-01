#!/usr/bin/env python3
"""Archon Marker sidecar — PDF → normalized Marker block-tree JSON (device-agnostic).

Port #2 substrate. Runs Marker and emits the nested block tree that the Rust parser
(`archon-ingest-ext::marker::parse_marker_json`) consumes: each node has
`{block_type, id, html, bbox, children}`, with `Page` nodes' `id` like `/page/N/...`.

DEVICE-AGNOSTIC (per the standing constraint): the device is auto-detected
cuda → mps → cpu, overridable with `--device` or the `TORCH_DEVICE` env var. The same
script therefore runs on a Mac (Metal/MPS), an NVIDIA host/WRAITH (CUDA), or CPU with no
code change — only the resolved device differs. Transport (local subprocess vs HTTP) is
chosen on the Rust side and is orthogonal to device.

Usage:
    archon_marker_sidecar.py <pdf> [--device cuda|mps|cpu] [--output out.json]
    archon_marker_sidecar.py --selftest [--output out.json]   # emits a fixture, no torch

`--selftest` emits a canned block tree WITHOUT importing torch/marker, so the Rust client
and JSON contract can be validated on a box where Marker/torch is not installed (e.g. WSL).
"""

import argparse
import json
import os
import sys


def resolve_device(explicit: str | None) -> str:
    """cuda → mps → cpu. `--device`/`TORCH_DEVICE` override auto-detection."""
    chosen = explicit or os.environ.get("TORCH_DEVICE")
    if chosen and chosen != "auto":
        return chosen
    try:
        import torch  # noqa: WPS433 (local import: keep --selftest torch-free)

        if torch.cuda.is_available():
            return "cuda"
        if getattr(torch.backends, "mps", None) is not None and torch.backends.mps.is_available():
            return "mps"
    except Exception:  # torch absent or broken → CPU is always valid
        pass
    return "cpu"


# A minimal block tree matching the Rust parser's expectations. Shapes the contract:
# Page id "/page/N/..." → 1-indexed page N+1; text blocks carry html + bbox; a Table
# block carries an HTML <table> the Rust side parses + gates.
SELFTEST_FIXTURE = {
    "block_type": "Document",
    "children": [
        {
            "block_type": "Page",
            "id": "/page/0/Page/0",
            "bbox": [0, 0, 612, 792],
            "children": [
                {"block_type": "SectionHeader", "id": "/page/0/SectionHeader/0",
                 "html": "<h1>On the Soul</h1>", "bbox": [72, 60, 400, 90], "children": []},
                {"block_type": "Text", "id": "/page/0/Text/1",
                 "html": "<p>The <i>energeia</i> of a living body.</p>",
                 "bbox": [72, 100, 540, 140], "children": []},
                {"block_type": "Table", "id": "/page/0/Table/2",
                 "html": "<table><tr><th>Year</th><th>N</th></tr>"
                         "<tr><td>2019</td><td>12</td></tr><tr><td>2020</td><td>8</td></tr>"
                         "<tr><td>2021</td><td>15</td></tr></table>",
                 "bbox": [72, 150, 540, 240], "children": []},
            ],
        },
        {
            "block_type": "Page",
            "id": "/page/1/Page/1",
            "bbox": [0, 0, 612, 792],
            "children": [
                {"block_type": "Text", "id": "/page/1/Text/0",
                 "html": "<p>Second page body.</p>", "bbox": [72, 60, 540, 100], "children": []},
                {"block_type": "Text", "id": "/page/1/Text/1",
                 "html": "1147a", "bbox": [300, 740, 330, 758], "children": []},
            ],
        },
    ],
}


def run_marker(pdf_path: str, device: str, page_range: "list[int] | None" = None) -> dict:
    """Run Marker with its JSON renderer and return the block tree as a dict.

    Marker's JSON renderer already emits `{block_type, id, html, bbox/polygon, children}`,
    which is exactly the Rust parser's contract. `TORCH_DEVICE` is set before importing
    Marker so its models load on the resolved device.

    `page_range` (0-indexed original-PDF page numbers) restricts the run to those pages; Marker
    keeps ABSOLUTE page ids (`/page/N/`), so a caller can process a big PDF in page-range chunks
    that each fit VRAM and concatenate the block streams without re-offsetting. `None` = whole doc.

    [CONFIRM on the Mac] against the installed Marker version: the converter/renderer import
    paths and whether boxes come back as `bbox` (used here) or `polygon` (map to a bbox).
    """
    os.environ["TORCH_DEVICE"] = device
    from marker.converters.pdf import PdfConverter
    from marker.models import create_model_dict

    config = {"page_range": page_range} if page_range is not None else {}
    converter = PdfConverter(
        artifact_dict=create_model_dict(),
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


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Archon Marker sidecar (device-agnostic).")
    parser.add_argument("pdf", nargs="?", help="path to the input PDF")
    parser.add_argument("--device", help="cuda|mps|cpu|auto (default/auto: cuda→mps→cpu)")
    parser.add_argument("--page-range", dest="page_range",
                        help="0-indexed inclusive page range 'START-END' (default: whole doc)")
    parser.add_argument("--output", help="write JSON here (default: stdout)")
    parser.add_argument("--selftest", action="store_true",
                        help="emit a fixture block tree without importing torch/marker")
    args = parser.parse_args(argv)

    device = resolve_device(args.device)
    page_range = None
    if args.page_range:
        try:
            start, end = (int(x) for x in args.page_range.split("-", 1))
        except ValueError:
            parser.error(f"--page-range must be 'START-END', got {args.page_range!r}")
        page_range = list(range(start, end + 1))
    if args.selftest:
        tree = SELFTEST_FIXTURE
    else:
        if not args.pdf:
            parser.error("a <pdf> path is required unless --selftest is given")
        sys.stderr.write(
            f"[archon-marker] device={device} pdf={args.pdf} page_range={args.page_range or 'all'}\n"
        )
        try:
            tree = run_marker(args.pdf, device, page_range)
        except Exception as exc:  # noqa: BLE001 — classify OOM, re-raise everything else
            if "out of memory" in str(exc).lower() or type(exc).__name__ == "OutOfMemoryError":
                # Structured OOM signal: the Rust caller (marker_source.rs) catches exit 42
                # (or this stderr line) and retries the document on CPU.
                sys.stderr.write(f"[archon-marker] OOM on device={device}: {exc}\n")
                return 42
            raise

    payload = json.dumps(tree, ensure_ascii=False)
    if args.output:
        with open(args.output, "w", encoding="utf-8") as fh:
            fh.write(payload)
    else:
        sys.stdout.write(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
