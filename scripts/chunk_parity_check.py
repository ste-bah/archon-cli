#!/usr/bin/env python3
"""Parity harness — run the Python reference `chunk_marker_json` on a fixture so its chunk
boundaries / page lineage can be diffed against the Rust `chunk_blocks` port (golden gate S-1).

The reference operates on already-parsed Marker JSON (no torch), so this runs anywhere.
Writes the reference chunks as JSON to stdout (and optionally --output).

Two invocations:
  - no arguments         → run the built-in fixture smoke test (unchanged, byte-for-byte)
  - --marker-json <path> → load a real Marker JSON dump and print the Python reference
                           chunk table (page_start, page_end, text_head, text_len) so PR-D
                           can diff it against the Rust port on real corpus PDFs.
"""
import argparse
import json
import os
import sys

# Reference lives in the god-agent tree; path is overridable for portability.
REF_DIR = os.environ.get(
    "CHUNKER_REF_DIR",
    "/home/dalton/projects/claudeflow-testing/scripts/ingest",
)
sys.path.insert(0, REF_DIR)
from markdown_chunker import chunk_marker_json  # noqa: E402

BIG1 = "a" * 3600  # ~900 tokens (chars/4) — exceeds TARGET_MIN, so a heading boundary splits

FIXTURE = {
    "block_type": "Document",
    "children": [
        {
            "block_type": "Page", "id": "/page/0/Page/0",
            "children": [
                {"block_type": "Text", "html": f"<p>{BIG1}</p>", "bbox": [10, 20, 500, 400]},
                {"block_type": "SectionHeader", "html": "<h2>Section Two</h2>", "bbox": [10, 410, 500, 440]},
            ],
        },
        {
            "block_type": "Page", "id": "/page/1/Page/1",
            "children": [
                {"block_type": "Text", "html": "<p>short tail body</p>", "bbox": [10, 20, 500, 60]},
            ],
        },
    ],
}


def _run_fixture() -> int:
    """Built-in fixture smoke test — the default, no-argument behavior (kept byte-for-byte)."""
    chunks = chunk_marker_json(FIXTURE)
    out = [
        {
            "page_start": c["page_start"],
            "page_end": c["page_end"],
            "text_head": c["text"][:24],
            "text_len": len(c["text"]),
            "bbox_pages": [b["page_num"] for b in c["bboxes"]],
        }
        for c in chunks
    ]
    print(json.dumps(out, indent=2))
    return 0


def _run_marker_json(path: str) -> int:
    """Load a real Marker JSON dump and print the Python reference chunk table.

    The dump is the parsed Marker document tree (the same shape `chunk_marker_json` consumes:
    a dict with `block_type`/`children`/`bbox`). Prints one row per reference chunk so PR-D can
    diff page lineage + chunk text against the Rust `chunk_blocks` port on real corpus PDFs.
    """
    with open(path, "r", encoding="utf-8") as fh:
        marker_json = json.load(fh)

    chunks = chunk_marker_json(marker_json)

    print(f"# reference chunks for {path}")
    print(f"# {len(chunks)} chunk(s) — columns: idx  page_start  page_end  text_len  text_head")
    print(f"{'idx':>3}  {'p_start':>7}  {'p_end':>5}  {'text_len':>8}  text_head")
    for i, c in enumerate(chunks):
        # Single-line, truncated head so the table stays readable on real prose.
        head = " ".join(c["text"][:60].split())
        print(f"{i:>3}  {c['page_start']:>7}  {c['page_end']:>5}  {len(c['text']):>8}  {head}")
    return 0


def main(argv=None) -> int:
    argv = sys.argv[1:] if argv is None else list(argv)
    # No arguments → preserve the original fixture invocation exactly (byte-for-byte).
    if not argv:
        return _run_fixture()

    parser = argparse.ArgumentParser(
        description="Run the Python reference chunker for Rust parity diffing.",
    )
    parser.add_argument(
        "--marker-json",
        metavar="PATH",
        required=True,
        help="Path to a real Marker JSON dump; prints the reference chunk table for it.",
    )
    args = parser.parse_args(argv)
    return _run_marker_json(args.marker_json)


if __name__ == "__main__":
    raise SystemExit(main())
