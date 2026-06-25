#!/usr/bin/env python3
"""Parity harness — run the Python reference `chunk_marker_json` on a fixture so its chunk
boundaries / page lineage can be diffed against the Rust `chunk_blocks` port (golden gate S-1).

The reference operates on already-parsed Marker JSON (no torch), so this runs anywhere.
Writes the reference chunks as JSON to stdout (and optionally --output).
"""
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


def main() -> int:
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


if __name__ == "__main__":
    raise SystemExit(main())
