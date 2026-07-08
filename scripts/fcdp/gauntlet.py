#!/usr/bin/env python3
"""FCDP sandbox — mechanical gates G-B(post) / G-C / G-D / G-F + exemplar overlap.
(Sandbox-Python; mechanical translation to Rust at M7 integration. G-A and the
substitution step live in the Rust `fcdp` binary.)

usage: gauntlet.py <substituted-draft> <pack.json> [--presub <pre-substitution-draft>]
       → JSON defect report, exit 1 on defects
"""
import json, re, sys, unicodedata
from pathlib import Path

draft_path, pack_path = Path(sys.argv[1]), Path(sys.argv[2])
presub = None
if "--presub" in sys.argv:
    presub = Path(sys.argv[sys.argv.index("--presub") + 1]).read_text()
draft = draft_path.read_text()
pack = json.loads(pack_path.read_text())
bank = json.loads((pack_path.parent / pack["p4b_bank_path"]).read_text())

defects, advisories = [], []

# Short scare-quote spans (<=3 words) that the FOUNDATION text itself uses in
# quotes (any quote style) are allowed — they are the author's own device.
_f = pack.get("p7_foundation", "")
_found_spans = set()
for m in re.findall(r'[\"\u201c``]{1,2}([^\"\u201d`\']{1,40}?)[\"\u201d\']{1,2}', _f):
    if len(m.split()) <= 3:
        _found_spans.add(m.strip().strip('.,;:').lower())
def _foundation_ok(span):
    return span.strip().strip('.,;:').lower() in _found_spans and len(span.split()) <= 3

# ── D2 protocol: quotes must enter by «Qnn» marker, never generated literally.
# Only the PRE-substitution draft can show this (post-sub, markers become quotes).
if presub is not None:
    for s in re.findall(r"``(.+?)''", presub, flags=re.S):
        if _foundation_ok(s):
            continue
        defects.append(f"G-B/D2: literal quoted span in pre-substitution draft "
                       f"(must enter via «Qnn»): ``{s[:60]}''")

# ── G-B (post-substitution): every quoted span must character-match a bank entry ──
bank_texts = {k: v["text"] for k, v in bank.items()}
spans = re.findall(r"``(.+?)''", draft, flags=re.S)
for s in spans:
    inner_ok = any(s.strip() == bt.strip("`'") or f"``{s}''" == bt for bt in bank_texts.values()) \
               or _foundation_ok(s)
    if not inner_ok:
        defects.append(f"G-B: quoted span not covered by bank: ``{s[:60]}...''" if len(s) > 60
                       else f"G-B: quoted span not covered by bank: ``{s}''")
# leftover markers = substitution incomplete
if re.search(r"«[A-Z]\d+[+@]?»", draft):
    defects.append("G-B: unsubstituted «Qnn» markers remain")
# straight-double-quote usage outside LaTeX markup
if re.search(r'(?<!\\)"', draft):
    advisories.append("G-B: straight double-quote characters present — verify not quotation use")
# unused ASSIGNED quotes (plan said they'd be used)
for qid in bank:
    if bank_texts[qid].strip("`'") not in draft and f"«{qid}" not in draft:
        defects.append(f"G-B: assigned quote {qid} unused in draft")

# ── exemplar leakage: any 8-gram shared with a P2b exemplar ──
# Bank-covered spans are excluded: quotation text legitimately recurs (the
# exemplar may cite the same locus the draft quotes via «Qnn» substitution).
def ngrams(text, n=8):
    w = re.sub(r"[^a-z0-9\s]", " ", text.lower()).split()
    return {" ".join(w[i:i+n]) for i in range(len(w) - n + 1)}
bank_grams = set()
for v in bank.values():
    # all rendered forms: bare text, text+cite («Qnn+»), cite alone («Qnn@»)
    for form in (v["text"], f'{v["text"]} {v.get("cite","")}', v.get("cite", "")):
        bank_grams |= ngrams(form)
d8 = ngrams(draft) - bank_grams
for ex in pack.get("p2b_exemplars", []):
    hits = d8 & (ngrams(ex["text"]) - bank_grams)
    if hits:
        defects.append(f"G-B/exemplar-leak ({ex['movement_type']}): {sorted(hits)[0][:70]}...")

# ── G-C: citation rigor — every paren-citation locus must trace to pack ──
known_loci = {q["locus"] for q in pack["p4a_quote_index"]}
known_srcs = " ".join(q["source"] + " " + q["locus"] for q in pack["p4a_quote_index"]) \
             + " " + " ".join(v.get("cite", "") for v in bank.values()) \
             + " " + pack["p7_foundation"] + " " + " ".join(e["content"] for e in pack["p5_evidence"])
_norm_locus = lambda x: x.replace("--", "-").replace("\u2013", "-")
known_srcs_n = _norm_locus(known_srcs)
for cite in re.findall(r"\(([^()]{2,60}?\d[^()]*?)\)", draft):
    tokens = [_norm_locus(t) for t in re.split(r"[,\s]+", cite) if re.search(r"\d", t)]
    if tokens and not any(t in known_srcs_n for t in tokens):
        defects.append(f"G-C: locus '({cite})' does not trace to pack (memory locus?)")
if "******" in draft:
    advisories.append("G-C: ****** placeholder(s) present — expected for missing sources; verify at handoff")

# ── G-D: terminology locks (greppable subset of the lock list) ──
GD = [
    (r"phantasmat", "retired stem 'phantasmat-'"),
    (r"\\textbf\{[^}]{2,40}\.\}", "bold run-in head"),
    (r"\b(she|he|his|her|hers)\b[^.]{0,40}\bplayer\b|\bplayer\b[^.]{0,60}\b(she|he|his|her|hers)\b", "gendered pronoun bound to 'the player'"),
    (r"supplies the [a-z ]*anchor", "'supplies the ... anchor' pattern"),
    (r"\b[Aa]s (discussed|noted|mentioned) (above|earlier|previously)\b", "explicit back-reference"),
    (r"[“”‘’]", "Unicode quote character"),
    (r"\bfaculty of (imagination|opinion|desire|perception)\b", "'faculty of X' instead of Greek term"),
]
gd_draft = draft
for bt in bank_texts.values():
    gd_draft = gd_draft.replace(bt.strip("`'"), " [BANK-QUOTE] ")
for pat, label in GD:
    m = re.search(pat, gd_draft)
    if m:
        defects.append(f"G-D: {label}: '{gd_draft[max(0,m.start()-20):m.end()+20]}'")
# P8 negative constraints, applied greppably
for c in pack.get("p8_negative_constraints", []):
    m = re.search(r"'([^']+)'", c)
    if m and re.search(re.escape(m.group(1)), draft, flags=re.I):
        defects.append(f"G-D/P8: banned phrase present: {m.group(1)}")

# ── G-F: degradation checklist (mechanical subset) ──
wc = len(draft.split())
lo, hi = pack["p1_task"]["target_words"]
if not (lo * 0.8 <= wc <= hi * 1.3):
    advisories.append(f"G-F: word count {wc} vs target [{lo},{hi}]")
nfkd = unicodedata.normalize("NFKD", draft)
if any(ord(ch) > 0x2500 for ch in nfkd):
    advisories.append("G-F: unusual Unicode blocks present")

report = {"defects": defects, "advisories": advisories, "pass": not defects}
print(json.dumps(report, indent=1))
sys.exit(0 if report["pass"] else 1)
