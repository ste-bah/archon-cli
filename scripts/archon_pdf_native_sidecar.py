#!/usr/bin/env python3
"""Native-coordinate block extraction for born-digital PDFs.

Reads the PDF's own glyph positions (no OCR, no GPU) and emits the SAME nested
block-tree JSON the Marker sidecar prints, so the Rust side parses it with the
existing `archon_ingest_ext::marker::parse_marker_str` — no new Rust parser.

Engines (auto-detected, both emit identical JSON):
  - pymupdf: `fitz` block/span extraction — real font sizes for header
    classification, `page.find_tables()` for tables, correct ligatures.
  - tsv:     `pdftotext -tsv` (poppler) — word rows grouped into paragraph
    blocks by (page_num, par_num, block_num); heuristic classification.

Shared post-processing (both engines):
  - de-hyphenation across line breaks (trailing '-' + lowercase continuation)
  - running-head strip: same normalized text in the top/bottom band on >= 3
    pages is dropped. Bare page numbers are KEPT as standalone blocks — the
    Rust side's locator extraction (layout::extract_locators) strips them from
    the body and stores them as PageNumber locators, same as the Marker path.
  - conservative 2-column reordering: applied only when the page has a
    high-confidence gutter (>= 3 blocks fully on each side, few spanners) AND
    the geometric order disagrees with the extractor's order. Poppler's own
    reading order is usually right; agreement is a no-op.
  - table detection (tsv engine only; pymupdf uses find_tables): a grid of
    short blocks with >= 3 aligned rows and >= 2 aligned columns becomes one
    `<table>` block. The Rust `is_real_table` gate rejects false positives.

Usage:
  archon_pdf_native_sidecar.py <pdf> [--engine auto|tsv|pymupdf] [--output F]
  archon_pdf_native_sidecar.py --selftest [--output F]

Exit codes: 0 ok; 2 usage; 3 extraction failed (caller falls back).
"""

from __future__ import annotations

import argparse
import html as html_mod
import json
import re
import shutil
import subprocess
import sys
from collections import Counter, defaultdict

# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------


class RawBlock:
    """One visual text block: bbox in PDF points (top-left origin) + lines of words."""

    __slots__ = ("page", "x0", "y0", "x1", "y1", "lines", "sizes", "kind", "html")

    def __init__(self, page):
        self.page = page  # 0-indexed
        self.x0 = self.y0 = float("inf")
        self.x1 = self.y1 = float("-inf")
        self.lines = []  # list[list[str]] — words per line
        self.sizes = []  # per-word glyph heights (tsv) or span sizes (pymupdf)
        self.kind = None  # classified later; "Table" blocks carry pre-built html
        self.html = None

    def add_word(self, text, x0, y0, x1, y1, size, line_key):
        if not self.lines or self.lines[-1][0] != line_key:
            self.lines.append((line_key, []))
        self.lines[-1][1].append(text)
        self.x0 = min(self.x0, x0)
        self.y0 = min(self.y0, y0)
        self.x1 = max(self.x1, x1)
        self.y1 = max(self.y1, y1)
        self.sizes.append(size)

    @property
    def bbox(self):
        return [self.x0, self.y0, self.x1, self.y1]

    @property
    def cx(self):
        return (self.x0 + self.x1) / 2.0

    def text(self):
        """Join lines with de-hyphenation: 'analy-' + 'sis' -> 'analysis'."""
        out = []
        for _key, words in self.lines:
            if not words:
                continue
            if out and out[-1].endswith("-") and len(out[-1]) > 1 and words[0][:1].islower():
                out[-1] = out[-1][:-1] + words[0]
                out.extend(words[1:])
            else:
                out.extend(words)
        return " ".join(out)

    def word_count(self):
        return sum(len(w) for _k, w in self.lines)

    def median_size(self):
        if not self.sizes:
            return 0.0
        s = sorted(self.sizes)
        return s[len(s) // 2]


# ---------------------------------------------------------------------------
# TSV engine (pdftotext -tsv)
# ---------------------------------------------------------------------------


def tsv_rows(lines):
    """Parse TSV lines -> (page_dims: {page0: (w, h)}, blocks: [RawBlock]).

    Level 1 rows carry page dims; level 5 rows with conf >= 0 are words.
    Grouping key for a block is (page_num, par_num, block_num) — block_num
    RESETS within par_num (verified against poppler 24.02 output).
    Line identity within a block is line_num (drives de-hyphenation).
    """
    page_dims = {}
    blocks = {}
    order = []  # grouping keys in first-seen (reading) order
    it = iter(lines)
    header = next(it, None)
    if header is None or not header.startswith("level"):
        raise ValueError("tsv: missing header row")
    for line in it:
        parts = line.rstrip("\n").split("\t")
        if len(parts) != 12:
            continue
        level, page_num, par_num, block_num, line_num = parts[0], parts[1], parts[2], parts[3], parts[4]
        try:
            left, top, width, height = (float(parts[6]), float(parts[7]), float(parts[8]), float(parts[9]))
            conf = float(parts[10])
        except ValueError:
            continue
        text = parts[11]
        page0 = int(page_num) - 1  # poppler tsv pages are 1-indexed
        if level == "1":
            page_dims[page0] = (width, height)
            continue
        if level != "5" or conf < 0 or not text or text.startswith("###"):
            continue
        key = (page0, int(par_num), int(block_num))
        if key not in blocks:
            blocks[key] = RawBlock(page0)
            order.append(key)
        blocks[key].add_word(
            text, left, top, left + width, top + height, height, int(line_num)
        )
    return page_dims, [blocks[k] for k in order]


def extract_tsv(pdf_path, pdftotext_bin="pdftotext"):
    """Run `pdftotext -tsv` on the whole document, streaming stdout line-by-line."""
    proc = subprocess.Popen(
        [pdftotext_bin, "-tsv", pdf_path, "-"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        errors="replace",
    )
    try:
        page_dims, blocks = tsv_rows(proc.stdout)
    finally:
        stderr = proc.stderr.read()
        proc.stdout.close()
        proc.stderr.close()
        proc.wait()
    if proc.returncode != 0:
        raise RuntimeError(f"pdftotext -tsv exited {proc.returncode}: {stderr.strip()}")
    return page_dims, blocks


# ---------------------------------------------------------------------------
# PyMuPDF engine
# ---------------------------------------------------------------------------


def extract_pymupdf(pdf_path):
    """fitz block extraction: real font sizes + find_tables. Same output shape as tsv."""
    import fitz  # noqa: PLC0415 — soft dependency, availability checked by caller

    page_dims = {}
    blocks = []
    doc = fitz.open(pdf_path)
    try:
        for page0, page in enumerate(doc):
            rect = page.rect
            page_dims[page0] = (rect.width, rect.height)
            table_bboxes = []
            try:
                for t in page.find_tables():
                    rows = t.extract()
                    clean = [["" if c is None else str(c).strip() for c in row] for row in rows]
                    if len(clean) >= 2 and any(any(c for c in row) for row in clean):
                        tb = RawBlock(page0)
                        x0, y0, x1, y1 = t.bbox
                        tb.x0, tb.y0, tb.x1, tb.y1 = x0, y0, x1, y1
                        tb.kind = "Table"
                        tb.html = table_html(clean)
                        tb.lines = [(0, ["[table]"])]
                        blocks.append(tb)
                        table_bboxes.append((x0, y0, x1, y1))
            except Exception as e:  # noqa: BLE001 — table finding must never sink the doc
                print(f"pdf-native: find_tables failed p{page0 + 1}: {e}", file=sys.stderr)

            for b in page.get_text("dict")["blocks"]:
                if b.get("type") != 0:
                    continue  # image block
                bx0, by0, bx1, by1 = b["bbox"]
                cx, cy = (bx0 + bx1) / 2, (by0 + by1) / 2
                if any(tx0 <= cx <= tx1 and ty0 <= cy <= ty1 for tx0, ty0, tx1, ty1 in table_bboxes):
                    continue  # cell text already captured by the table block
                rb = RawBlock(page0)
                for li, line in enumerate(b.get("lines", [])):
                    line_text = "".join(s.get("text", "") for s in line.get("spans", []))
                    words = line_text.split()
                    if not words:
                        continue
                    lx0, ly0, lx1, ly1 = line["bbox"]
                    sizes = [s.get("size", 0.0) for s in line.get("spans", []) if s.get("text", "").strip()]
                    size = sorted(sizes)[len(sizes) // 2] if sizes else 0.0
                    for w in words:
                        rb.add_word(w, lx0, ly0, lx1, ly1, size, li)
                if rb.lines:
                    blocks.append(rb)
    finally:
        doc.close()
    return page_dims, blocks


# ---------------------------------------------------------------------------
# Shared post-processing
# ---------------------------------------------------------------------------

PAGE_NUM_RE = re.compile(r"^\d{1,4}$")
CAPTION_RE = re.compile(r"^(Figure|Fig\.|Table|Exhibit|Chart)\b", re.IGNORECASE)
LIST_RE = re.compile(r"^([•◦⁃\-\*]|\d{1,2}[.)]|[a-z][.)])\s")
TERMINAL_PUNCT = (".", "!", "?", ";", ":", ",")


def doc_body_size(blocks):
    """Text-length-weighted median glyph size across the document = body size."""
    weighted = []
    for b in blocks:
        if b.kind == "Table":
            continue
        n = max(1, b.word_count())
        weighted.append((b.median_size(), n))
    if not weighted:
        return 0.0
    total = sum(n for _s, n in weighted)
    acc = 0
    for s, n in sorted(weighted):
        acc += n
        if acc * 2 >= total:
            return s
    return weighted[-1][0]


def strip_running_heads(blocks, page_dims, min_pages=3):
    """Drop running decorations; keep bare page numbers (Rust locator capture).

    Two confirmation rules, both requiring the same digit-masked text on
    >= min_pages DISTINCT pages:
      - band rule: any text in the top-6% / bottom-8% band (classic running head)
      - repeat rule: any >= 3-word block ANYWHERE on the page (rotated margin
        watermarks — e.g. "Downloaded by [library] at <time>" — sit mid-page)
    """
    def in_band(b):
        w_h = page_dims.get(b.page)
        if not w_h:
            return False
        _w, h = w_h
        return b.y0 < h * 0.06 or b.y1 > h * 0.92

    norm = lambda t: re.sub(r"\d+", "#", t.strip().lower())  # noqa: E731
    band_pages = defaultdict(set)
    repeat_pages = defaultdict(set)
    for b in blocks:
        t = b.text().strip()
        if PAGE_NUM_RE.match(t):
            continue
        if in_band(b):
            band_pages[norm(t)].add(b.page)
        if b.word_count() >= 3:
            repeat_pages[norm(t)].add(b.page)
    confirmed = {t for t, pages in band_pages.items() if len(pages) >= min_pages}
    confirmed |= {t for t, pages in repeat_pages.items() if len(pages) >= min_pages}
    out = []
    for b in blocks:
        t = b.text().strip()
        if not PAGE_NUM_RE.match(t) and norm(t) in confirmed:
            continue
        out.append(b)
    return out


def order_two_column(page_blocks, page_w):
    """Conservative 2-column reorder. Returns the input list unchanged unless the
    page has a high-confidence gutter AND geometric order differs from input.
    Bare page numbers are column-neutral (a centered folio straddles any gutter);
    they re-attach at their y-position after the columns are ordered."""
    if len(page_blocks) < 6:
        return page_blocks
    neutral = [b for b in page_blocks if PAGE_NUM_RE.match(b.text().strip())]
    body = [b for b in page_blocks if id(b) not in {id(n) for n in neutral}]
    if len(body) < 6:
        return page_blocks
    for frac in (0.5, 0.45, 0.55, 0.4, 0.6, 0.35, 0.65):
        s = page_w * frac
        left = [b for b in body if b.x1 <= s]
        right = [b for b in body if b.x0 >= s]
        span = [b for b in body if b.x0 < s < b.x1]
        if len(left) >= 3 and len(right) >= 3 and len(span) <= max(1, len(body) // 5):
            break
    else:
        return page_blocks
    span = sorted(span + neutral, key=lambda b: b.y0)
    # Full-width (spanning) blocks split the page into vertical segments; within
    # each segment: left column top->bottom, then right column top->bottom.
    ordered = []
    seg_top = float("-inf")
    for sp in sorted(span, key=lambda b: b.y0) + [None]:
        seg_bot = sp.y0 if sp is not None else float("inf")
        seg_l = sorted((b for b in left if seg_top <= b.y0 < seg_bot), key=lambda b: b.y0)
        seg_r = sorted((b for b in right if seg_top <= b.y0 < seg_bot), key=lambda b: b.y0)
        ordered.extend(seg_l)
        ordered.extend(seg_r)
        if sp is not None:
            ordered.append(sp)
            seg_top = sp.y0
    if [id(b) for b in ordered] == [id(b) for b in page_blocks]:
        return page_blocks
    return ordered


def detect_tsv_tables(page_blocks):
    """Grid detection for the tsv engine: a run of short blocks forming >= 3
    aligned rows x >= 2 aligned columns becomes ONE Table block. Cells are
    consumed; other blocks pass through in order."""
    short = [b for b in page_blocks if b.kind is None and b.word_count() <= 4 and len(b.lines) == 1]
    if len(short) < 6:
        return page_blocks
    # Split candidates into vertical groups on big y-gaps (stray shorts elsewhere
    # on the page must not glue into the grid).
    short.sort(key=lambda b: (b.y0, b.x0))
    groups, cur = [], [short[0]]
    for b in short[1:]:
        if b.y0 - cur[-1].y0 > 40:
            groups.append(cur)
            cur = [b]
        else:
            cur.append(b)
    groups.append(cur)

    consumed = set()
    tables = []
    for g in groups:
        if len(g) < 6:
            continue
        rows = cluster(sorted(b.y0 for b in g), tol=3.0)
        cols = cluster(sorted(b.x0 for b in g), tol=5.0)
        if len(rows) < 3 or len(cols) < 2 or len(g) < len(rows) * len(cols) * 0.7:
            continue
        grid = defaultdict(dict)
        for b in g:
            r = nearest(rows, b.y0)
            c = nearest(cols, b.x0)
            grid[r][c] = (grid[r].get(c, "") + " " + b.text()).strip()
            consumed.add(id(b))
        cells = [[grid[r].get(c, "") for c in sorted(cols)] for r in sorted(rows)]
        tb = RawBlock(g[0].page)
        tb.x0 = min(b.x0 for b in g)
        tb.y0 = min(b.y0 for b in g)
        tb.x1 = max(b.x1 for b in g)
        tb.y1 = max(b.y1 for b in g)
        tb.kind = "Table"
        tb.html = table_html(cells)
        tb.lines = [(0, ["[table]"])]
        tables.append(tb)

    if not tables:
        return page_blocks
    out, inserted = [], set()
    for b in page_blocks:
        if id(b) in consumed:
            for t in tables:  # insert each table at its first cell's position
                if id(t) not in inserted and t.page == b.page and t.y0 <= b.y0 + 3:
                    out.append(t)
                    inserted.add(id(t))
            continue
        out.append(b)
    for t in tables:
        if id(t) not in inserted:
            out.append(t)
    return out


def cluster(sorted_vals, tol):
    """1-D clustering: representative value per group of near-equal coordinates."""
    reps = []
    for v in sorted_vals:
        if not reps or v - reps[-1] > tol:
            reps.append(v)
    return reps


def nearest(reps, v):
    return min(reps, key=lambda r: abs(r - v))


def classify(blocks, body_size):
    """Assign a Marker block_type to every non-table block."""
    for b in blocks:
        if b.kind is not None:
            continue
        t = b.text().strip()
        if PAGE_NUM_RE.match(t):
            b.kind = "Text"  # standalone number → Rust locator capture strips it
        elif CAPTION_RE.match(t):
            b.kind = "Caption"
        elif LIST_RE.match(t):
            b.kind = "ListItem"
        elif (
            len(t) < 100
            and not t.endswith(TERMINAL_PUNCT)
            and body_size > 0
            and b.median_size() > body_size * 1.15
        ):
            b.kind = "SectionHeader"
        else:
            b.kind = "Text"


def table_html(cells):
    esc = html_mod.escape
    rows = "".join(
        "<tr>" + "".join(f"<td>{esc(c)}</td>" for c in row) + "</tr>" for row in cells
    )
    return f"<table>{rows}</table>"


def block_html(b, body_size):
    if b.kind == "Table":
        return b.html
    text = html_mod.escape(b.text())
    if b.kind == "SectionHeader":
        tag = "h1" if body_size > 0 and b.median_size() > body_size * 1.4 else "h2"
        return f"<{tag}>{text}</{tag}>"
    return f"<p>{text}</p>"


def to_marker_json(page_dims, blocks):
    """Emit the Marker-compatible nested tree (Document -> Page -> leaf blocks)."""
    by_page = defaultdict(list)
    for b in blocks:
        by_page[b.page].append(b)
    body_size = doc_body_size(blocks)
    pages = []
    all_pages = sorted(set(page_dims) | set(by_page))
    for p in all_pages:
        w, h = page_dims.get(p, (612.0, 792.0))
        children = []
        for i, b in enumerate(by_page.get(p, [])):
            children.append(
                {
                    "block_type": b.kind,
                    "id": f"/page/{p}/{b.kind}/{i}",
                    "html": block_html(b, body_size),
                    "bbox": [round(v, 2) for v in b.bbox],
                    "children": [],
                }
            )
        pages.append(
            {
                "block_type": "Page",
                "id": f"/page/{p}/Page/{p}",
                "bbox": [0, 0, round(w, 2), round(h, 2)],
                "children": children,
            }
        )
    return {"block_type": "Document", "children": pages}


def run_pipeline(page_dims, blocks, engine):
    blocks = strip_running_heads(blocks, page_dims)
    if engine == "tsv":
        by_page = defaultdict(list)
        for b in blocks:
            by_page[b.page].append(b)
        rebuilt = []
        for p in sorted(by_page):
            rebuilt.extend(detect_tsv_tables(by_page[p]))
        blocks = rebuilt
    classify(blocks, doc_body_size(blocks))
    by_page = defaultdict(list)
    for b in blocks:
        by_page[b.page].append(b)
    ordered = []
    for p in sorted(by_page):
        w = page_dims.get(p, (612.0, 792.0))[0]
        ordered.extend(order_two_column(by_page[p], w))
    return to_marker_json(page_dims, ordered)


# ---------------------------------------------------------------------------
# Selftest
# ---------------------------------------------------------------------------


def selftest_tsv_lines():
    """Synthetic 3-page TSV: title header, 2 columns (deliberately interleaved to
    exercise the reorder), a hyphenated line break, running heads on 3 pages,
    bare page numbers, and a 3x2 grid on page 2."""
    rows = ["level\tpage_num\tpar_num\tblock_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext"]
    n = {"par": 0}

    def page(p, w=612.0, h=792.0):
        rows.append(f"1\t{p}\t0\t0\t0\t0\t0\t0\t{w}\t{h}\t-1\t###PAGE###")

    def block(p, words_lines, x, y, wordw=40.0, wh=10.0, line_h=14.0):
        n["par"] += 1
        par = n["par"]
        for li, words in enumerate(words_lines):
            wx = x
            for wi, w in enumerate(words):
                rows.append(
                    f"5\t{p}\t{par}\t0\t{li}\t{wi}\t{wx:.2f}\t{y + li * line_h:.2f}\t{wordw:.2f}\t{wh:.2f}\t100\t{w}"
                )
                wx += wordw + 5

    # Page 1 — running head, title (big glyphs), interleaved 2-column body, page number.
    page(1)
    block(1, [["Journal", "of", "Testing"]], 200, 30)
    block(1, [["On", "Native", "Extraction"]], 150, 90, wordw=90, wh=22)
    block(1, [["The", "left", "column", "begins", "the", "analy-"], ["sis", "of", "position."]], 72, 150, wordw=32.0)
    block(1, [["Right", "column", "starts", "here", "instead."]], 330, 150)
    block(1, [["Left", "middle", "paragraph", "content", "continues."]], 72, 300)
    block(1, [["Right", "middle", "paragraph", "text", "continues."]], 330, 300)
    block(1, [["Left", "bottom", "paragraph", "concludes", "column."]], 72, 450)
    block(1, [["Right", "bottom", "paragraph", "ends", "page."]], 330, 450)
    block(1, [["1"]], 300, 762, wordw=10)
    # Page 2 — running head, a paragraph, a 3x2 grid, page number.
    page(2)
    block(2, [["Journal", "of", "Testing"]], 200, 30)
    block(2, [["Results", "appear", "in", "the", "table", "below."]], 72, 120)
    for r, (a, b) in enumerate([("Year", "N"), ("2019", "12"), ("2020", "8")]):
        block(2, [[a]], 100, 200 + r * 20, wordw=30)
        block(2, [[b]], 200, 200 + r * 20, wordw=30)
    block(2, [["2"]], 300, 762, wordw=10)
    # Page 3 — running head, a paragraph, page number.
    page(3)
    block(3, [["Journal", "of", "Testing"]], 200, 30)
    block(3, [["Closing", "discussion", "paragraph", "on", "page", "three."]], 72, 120)
    block(3, [["3"]], 300, 762, wordw=10)
    return rows


def selftest():
    page_dims, blocks = tsv_rows(iter(selftest_tsv_lines()))
    tree = run_pipeline(page_dims, blocks, engine="tsv")

    def leaves(t):
        out = []
        for pg in t["children"]:
            out.extend(pg["children"])
        return out

    ls = leaves(tree)
    texts = [re.sub(r"<[^>]+>", "", l["html"]) for l in ls]
    fail = []
    if any("Journal of Testing" in t for t in texts):
        fail.append("running head not stripped")
    if not any(l["block_type"] == "SectionHeader" and "Native Extraction" in l["html"] for l in ls):
        fail.append("title not classified as SectionHeader")
    if not any("analysis of position." in t for t in texts):
        fail.append("hyphenated line break not merged")
    if "1" not in texts or "2" not in texts:
        fail.append("bare page numbers must be KEPT (locator capture is Rust-side)")
    tables = [l for l in ls if l["block_type"] == "Table"]
    if len(tables) != 1 or "<td>Year</td>" not in tables[0]["html"] or "<td>2020</td>" not in tables[0]["html"]:
        fail.append("3x2 grid not detected as a Table")
    p1 = [re.sub(r"<[^>]+>", "", l["html"]) for l in tree["children"][0]["children"]]
    body = [t for t in p1 if t.startswith(("The left", "Left", "Right"))]
    if body != [
        "The left column begins the analysis of position.",
        "Left middle paragraph content continues.",
        "Left bottom paragraph concludes column.",
        "Right column starts here instead.",
        "Right middle paragraph text continues.",
        "Right bottom paragraph ends page.",
    ]:
        fail.append(f"2-column reorder wrong: {body}")
    for l in ls:
        if not (isinstance(l["bbox"], list) and len(l["bbox"]) == 4 and l["bbox"][2] > l["bbox"][0]):
            fail.append(f"degenerate bbox on {l['id']}")
            break
    if fail:
        for f in fail:
            print(f"SELFTEST FAIL: {f}", file=sys.stderr)
        return None
    print("SELFTEST OK", file=sys.stderr)
    return tree


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("pdf", nargs="?", help="path to a born-digital PDF")
    ap.add_argument("--engine", choices=["auto", "tsv", "pymupdf"], default="auto")
    ap.add_argument("--output", help="write JSON here instead of stdout")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--device", help=argparse.SUPPRESS)  # marker-sidecar arg parity; ignored
    args = ap.parse_args()

    if args.selftest:
        tree = selftest()
        if tree is None:
            return 1
    else:
        if not args.pdf:
            ap.error("pdf path required unless --selftest")
        engine = args.engine
        if engine == "auto":
            # tsv (poppler) is the DEFAULT: it reproduces the flat pdftotext text
            # byte-for-byte (same library), which the corpus baseline was ingested
            # with — verbatim citation matching depends on that parity. PyMuPDF
            # renders combining diacritics differently (pathē -> "path¯e"), which
            # would break Greek-transliteration quote matching; it stays opt-in
            # via --engine pymupdf for corpora that need font-metadata features.
            engine = "tsv"
        if engine == "tsv" and shutil.which("pdftotext") is None:
            print("pdf-native: pdftotext not found (install poppler-utils)", file=sys.stderr)
            return 3
        try:
            if engine == "pymupdf":
                page_dims, blocks = extract_pymupdf(args.pdf)
            else:
                page_dims, blocks = extract_tsv(args.pdf)
            tree = run_pipeline(page_dims, blocks, engine)
        except Exception as e:  # noqa: BLE001 — any failure → non-zero, caller falls back
            print(f"pdf-native: extraction failed ({engine}): {e}", file=sys.stderr)
            return 3
        print(f"pdf-native: engine={engine}", file=sys.stderr)

    out = json.dumps(tree, ensure_ascii=False)
    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(out)
    else:
        print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
