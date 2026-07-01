#!/usr/bin/env python3
"""Adoption #2 corpus dry-run oracle (plan work-item #5).

Validates the Rust coverage classifier's ppi-proxy against an INDEPENDENT ground truth
(pypdfium2 image placement rects), and surfaces aspect-vs-coverage divergences across the corpus.

Per image the Rust code computes:
    coverage_proxy = (px_w * 72 / x_ppi) / page_w_pt * (px_h * 72 / y_ppi) / page_h_pt   [poppler ppi]
The oracle compares that to:
    coverage_true  = (rect_w_pt / page_w_pt) * (rect_h_pt / page_h_pt)                   [pdfium rect]
using the SAME page size for both, so the ONLY thing under test is poppler-ppi vs pdfium-rect.

Tolerance ±0.02: poppler reports ppi as an INTEGER, so the proxy has a quantization floor
(~0.2% at 241 ppi, larger for small/low-ppi images). A per-page diff beyond ±0.02 is the real
poppler quirk the oracle exists to catch.

It also replicates BOTH classifier verdicts (aspect + coverage) to report corpus-wide A/B divergence.
Usage: coverage_oracle.py <pdf-or-dir> [...]
"""
import subprocess
import sys
from pathlib import Path

import pypdfium2 as pdfium

PAGE_SCAN_COVERAGE = 0.80
SCANNED_BOOK_FRACTION = 0.70
MIN_USABLE_PPI = 10
TOL = 0.02


def pdfimages_entries(path):
    """[(page, px_w, px_h, x_ppi, y_ppi)] from `pdfimages -list` (image rows only)."""
    try:
        out = subprocess.run(
            ["pdfimages", "-list", str(path)],
            capture_output=True, text=True, timeout=120,
        ).stdout
    except Exception as e:  # noqa: BLE001
        return None, f"pdfimages failed: {e}"
    rows = []
    for line in out.splitlines():
        c = line.split()
        if len(c) < 14 or not c[0].isdigit():
            continue
        if c[2] != "image":  # skip smask/stencil rows
            continue
        try:
            page, w, h = int(c[0]), int(c[3]), int(c[4])
            xppi = int(c[12]) if c[12].lstrip("-").isdigit() else None
            yppi = int(c[13]) if c[13].lstrip("-").isdigit() else None
        except (ValueError, IndexError):
            continue
        rows.append((page, w, h, xppi, yppi))
    return rows, None


def is_page_scale_px(w, h):
    """Aspect page-scale test replicated from Rust: large (min side >= 1000) AND page-shaped (1.2..1.7)."""
    if w == 0 or h == 0:
        return False
    large = min(w, h) >= 1000
    ratio = max(w, h) / min(w, h)
    return large and (1.2 <= ratio <= 1.7)


def analyze(path):
    entries, err = pdfimages_entries(path)
    if err:
        return {"path": path, "error": err}
    try:
        doc = pdfium.PdfDocument(str(path))
    except Exception as e:  # noqa: BLE001
        return {"path": path, "error": f"pypdfium2 open failed: {e}"}

    n_pages = len(doc)
    page_size = {}          # 1-based page -> (w_pt, h_pt)
    pdfium_rects = {}       # 1-based page -> [ (w_pt, h_pt) ]
    for i in range(n_pages):
        page = doc[i]
        w, h = page.get_size()
        page_size[i + 1] = (w, h)
        rects = []
        for obj in page.get_objects():
            if obj.type == pdfium.raw.FPDF_PAGEOBJ_IMAGE:
                l, b, r, t = obj.get_pos()  # bottom-left origin; abs() → neutral for w/h
                rects.append((abs(r - l), abs(t - b)))
        pdfium_rects[i + 1] = rects

    # Per-page coverage from BOTH sources (sum of image coverages, capped at 1.0).
    proxy_page = {}
    true_page = {}
    deferred = 0
    for (page, w, h, xppi, yppi) in entries:
        pw, ph = page_size.get(page, (0, 0))
        if pw <= 0 or ph <= 0:
            deferred += 1
            continue
        if not xppi or not yppi or xppi < MIN_USABLE_PPI or yppi < MIN_USABLE_PPI:
            deferred += 1
            # aspect fallback contribution, mirroring the Rust deferral
            proxy_page[page] = proxy_page.get(page, 0.0) + (1.0 if is_page_scale_px(w, h) else 0.0)
            continue
        dw = w * 72.0 / xppi
        dh = h * 72.0 / yppi
        proxy_page[page] = proxy_page.get(page, 0.0) + (dw / pw) * (dh / ph)
    for page, rects in pdfium_rects.items():
        pw, ph = page_size[page]
        s = sum((rw / pw) * (rh / ph) for (rw, rh) in rects if pw > 0 and ph > 0)
        if s:
            true_page[page] = s

    proxy_page = {p: min(v, 1.0) for p, v in proxy_page.items()}
    true_page = {p: min(v, 1.0) for p, v in true_page.items()}

    # Per-page proxy-vs-truth diff on pages BOTH measured (the ppi-proxy validation).
    common = sorted(set(proxy_page) & set(true_page))
    diffs = [(p, abs(proxy_page[p] - true_page[p])) for p in common]
    max_diff = max((d for _, d in diffs), default=0.0)
    over = [(p, d) for p, d in diffs if d > TOL]

    def scanned(page_cov):
        scans = sum(1 for v in page_cov.values() if v >= PAGE_SCAN_COVERAGE)
        return (n_pages > 0 and scans / n_pages >= SCANNED_BOOK_FRACTION), scans

    proxy_scanned, proxy_scans = scanned(proxy_page)
    true_scanned, true_scans = scanned(true_page)

    # Aspect verdict (px-only).
    per_page_aspect = {}
    for (page, w, h, _x, _y) in entries:
        pc, tot = per_page_aspect.get(page, (0, 0))
        per_page_aspect[page] = (pc + (1 if is_page_scale_px(w, h) else 0), tot + 1)
    aspect_scans = sum(1 for (pc, tot) in per_page_aspect.values() if pc == 1 and tot == 1)
    aspect_scanned = n_pages > 0 and aspect_scans / n_pages >= SCANNED_BOOK_FRACTION

    return {
        "path": path, "pages": n_pages, "images": len(entries), "deferred": deferred,
        "max_page_diff": max_diff, "pages_over_tol": over,
        "proxy_scanned": proxy_scanned, "proxy_scans": proxy_scans,
        "true_scanned": true_scanned, "true_scans": true_scans,
        "aspect_scanned": aspect_scanned, "aspect_scans": aspect_scans,
    }


def collect(args):
    pdfs = []
    for a in args:
        p = Path(a)
        if p.is_dir():
            pdfs += sorted(p.rglob("*.pdf"))
        elif p.suffix.lower() == ".pdf":
            pdfs.append(p)
    return pdfs


def main():
    pdfs = collect(sys.argv[1:])
    if not pdfs:
        print("usage: coverage_oracle.py <pdf-or-dir> [...]", file=sys.stderr)
        sys.exit(2)
    print(f"# oracle over {len(pdfs)} PDF(s)  (tol ±{TOL})\n")
    hdr = f"{'proxyVtrue':>10} {'aspect':>7} {'cov(px)':>8} {'cov(true)':>9}  doc"
    print(hdr)
    print("-" * len(hdr))
    proxy_quirks, ab_divergent, errors = [], [], []
    for path in pdfs:
        r = analyze(path)
        name = Path(r["path"]).name
        if "error" in r:
            errors.append((name, r["error"]))
            print(f"{'ERR':>10} {'':>7} {'':>8} {'':>9}  {name}  ({r['error']})")
            continue
        flag = "  <== proxy quirk" if r["max_page_diff"] > TOL else ""
        ab = ""
        if r["aspect_scanned"] != r["proxy_scanned"]:
            ab = "  <== A/B DIVERGE"
            ab_divergent.append(r)
        if r["max_page_diff"] > TOL:
            proxy_quirks.append(r)
        print(f"{r['max_page_diff']:>10.4f} {str(r['aspect_scanned']):>7} "
              f"{str(r['proxy_scanned']):>8} {str(r['true_scanned']):>9}  {name}{flag}{ab}")
    print("\n================ SUMMARY ================")
    print(f"PDFs analyzed:            {len(pdfs) - len(errors)}")
    print(f"Errors:                   {len(errors)}")
    print(f"Proxy quirks (>{TOL}):     {len(proxy_quirks)}")
    for r in proxy_quirks:
        worst = max(r["pages_over_tol"], key=lambda x: x[1]) if r["pages_over_tol"] else (0, 0)
        print(f"   - {Path(r['path']).name}: max_diff={r['max_page_diff']:.4f} "
              f"(worst page {worst[0]} Δ{worst[1]:.4f}, {len(r['pages_over_tol'])} pages over)")
    print(f"Aspect vs coverage(px) divergences: {len(ab_divergent)}")
    for r in ab_divergent:
        print(f"   - {Path(r['path']).name}: aspect={r['aspect_scanned']} "
              f"coverage={r['proxy_scanned']} (aspect_scans={r['aspect_scans']} "
              f"cov_scans={r['proxy_scans']}/{r['pages']})")
    print("========================================")


if __name__ == "__main__":
    main()
