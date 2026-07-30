#!/usr/bin/env python3
"""Cross-device Marker bbox jitter diff (the PR-D measurement).

Run the SAME PDF through the Marker sidecar on multiple devices (e.g. RTX 5090 CUDA,
RTX 5070 CUDA, Apple MPS, CPU), dump the JSON each time, then:

    bbox_jitter_diff.py a.json b.json [c.json ...] \\
        [--labels cuda5090,cuda5070,mps,cpu] [--buckets 1,2,4,8] [--json]

It aligns blocks across dumps by their Marker id (``/page/N/Type/M``), computes the
per-coordinate absolute-delta distribution across every pair of dumps, and — the part
that actually drives the decision — reports, for each candidate integer quantization
bucket (px), the fraction of blocks whose bbox quantizes IDENTICALLY across all dumps.

That answers the open question from the plan: can rounding bbox coordinates before
hashing make ``spatial_hash`` device-independent, and at what bucket granularity / residual
mismatch rate? (See plans/archon-ingestion-port-finish-plan-2026-06-30.md §3 / PR-D.)

Notes
-----
* Stdlib only.
* Handles ``bbox`` ([x0,y0,x1,y1]) or ``polygon`` ([[x,y], ...] -> min/max bbox).
* Misaligned ids (a block present in one dump but not another, or a structural ordering
  difference) are reported separately — those are NOT jitter, they are a bigger problem,
  so they are surfaced loudly.
* Run the SAME device against itself twice first to establish the within-device noise
  floor; if that is non-zero, surya is nondeterministic even locally and no quantization
  fully fixes re-ingestion.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from itertools import combinations


def _polygon_to_bbox(poly):
    xs = [p[0] for p in poly]
    ys = [p[1] for p in poly]
    return [min(xs), min(ys), max(xs), max(ys)]


def collect_bboxes(node, out):
    """Recursively map Marker block id -> [x0, y0, x1, y1]."""
    if isinstance(node, dict):
        bbox = node.get("bbox")
        if bbox is None and isinstance(node.get("polygon"), list) and node["polygon"]:
            bbox = _polygon_to_bbox(node["polygon"])
        bid = node.get("id")
        if bid is not None and isinstance(bbox, list) and len(bbox) == 4:
            out[bid] = [float(c) for c in bbox]
        for child in node.get("children", []) or []:
            collect_bboxes(child, out)
    elif isinstance(node, list):
        for child in node:
            collect_bboxes(child, out)
    return out


def load(path):
    with open(path, "r", encoding="utf-8") as fh:
        return collect_bboxes(json.load(fh), {})


def percentile(values, pct):
    if not values:
        return 0.0
    s = sorted(values)
    k = max(0, min(len(s) - 1, int(round((pct / 100.0) * (len(s) - 1)))))
    return s[k]


def quantize(bbox, bucket):
    return tuple(round(c / bucket) for c in bbox)


def main(argv):
    ap = argparse.ArgumentParser(description="Cross-device Marker bbox jitter diff.")
    ap.add_argument("dumps", nargs="+", help="2+ Marker JSON dumps of the SAME PDF")
    ap.add_argument("--labels", help="comma-separated labels (default: filenames)")
    ap.add_argument(
        "--buckets",
        default="1,2,4,8",
        help="integer px quantization buckets to test (default 1,2,4,8)",
    )
    ap.add_argument("--json", action="store_true", help="emit a machine-readable summary")
    args = ap.parse_args(argv)

    if len(args.dumps) < 2:
        ap.error("need at least 2 dumps to diff")

    labels = (
        args.labels.split(",")
        if args.labels
        else [p.rsplit("/", 1)[-1] for p in args.dumps]
    )
    if len(labels) != len(args.dumps):
        ap.error("--labels count must match the number of dumps")
    buckets = [int(b) for b in args.buckets.split(",")]

    maps = [load(p) for p in args.dumps]
    id_sets = [set(m) for m in maps]
    common = set.intersection(*id_sets)
    union = set.union(*id_sets)
    unmatched = sorted(union - common)

    # Pairwise per-coordinate absolute deltas over commonly-aligned blocks.
    deltas = []
    pair_max = {}
    for (i, j) in combinations(range(len(maps)), 2):
        pm = 0.0
        for bid in common:
            a, b = maps[i][bid], maps[j][bid]
            for k in range(4):
                d = abs(a[k] - b[k])
                deltas.append(d)
                pm = max(pm, d)
        pair_max[f"{labels[i]} vs {labels[j]}"] = pm

    # Quantization unify-rate: fraction of common blocks whose quantized bbox is identical
    # across ALL dumps at a given bucket.
    unify = {}
    for bucket in buckets:
        same = 0
        for bid in common:
            q = {quantize(m[bid], bucket) for m in maps}
            if len(q) == 1:
                same += 1
        unify[bucket] = (same / len(common)) if common else 1.0

    summary = {
        "dumps": dict(zip(labels, args.dumps)),
        "blocks_per_dump": {labels[i]: len(maps[i]) for i in range(len(maps))},
        "common_blocks": len(common),
        "unmatched_ids": len(unmatched),
        "delta_max": max(deltas) if deltas else 0.0,
        "delta_p99": percentile(deltas, 99),
        "delta_p50": statistics.median(deltas) if deltas else 0.0,
        "delta_mean": statistics.fmean(deltas) if deltas else 0.0,
        "pair_max": pair_max,
        "quantize_unify_rate": unify,
    }

    if args.json:
        print(json.dumps(summary, indent=2))
        return 0

    print(f"dumps compared : {len(maps)}  ({', '.join(labels)})")
    print(f"blocks/dump    : {summary['blocks_per_dump']}")
    print(f"aligned blocks : {len(common)} of {len(union)} union")
    if unmatched:
        print(f"!! UNMATCHED ids: {len(unmatched)} (structural divergence, NOT jitter) — e.g. {unmatched[:5]}")
    print()
    print("per-coordinate absolute delta (px) across all pairs:")
    print(f"  max  {summary['delta_max']:.4f}")
    print(f"  p99  {summary['delta_p99']:.4f}")
    print(f"  p50  {summary['delta_p50']:.4f}")
    print(f"  mean {summary['delta_mean']:.4f}")
    print("  worst pair:", max(pair_max.items(), key=lambda kv: kv[1]) if pair_max else "n/a")
    print()
    print("quantization unify-rate (blocks identical across ALL dumps after rounding):")
    for bucket in buckets:
        print(f"  bucket={bucket:>2}px  ->  {unify[bucket] * 100:6.2f}% unify")
    best = next((b for b in buckets if unify[b] >= 0.999), None)
    if best is not None:
        print(f"\n=> bucket={best}px unifies >=99.9% of blocks: spatial_hash quantization is viable.")
    else:
        print("\n=> no tested bucket reaches 99.9%: geometry stays verify-by-recompute-only "
              "(use text/source hash for cross-machine identity).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
