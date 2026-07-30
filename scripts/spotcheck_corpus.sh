#!/usr/bin/env bash
# ============================================================================
# Post-flip corpus spot-check for archon.
# Proves each ingestion improvement is present AND used on real corpus data.
# Run from the archon-cli repo root, AFTER `docs ingest` + `docs index`.
#
#   bash scripts/spotcheck_corpus.sh [SAMPLE_N]
#
# Self-driving: it derives a real verbatim phrase from a search hit, so no
# manual phrase entry is needed. Reviewed by a human afterwards, not a gate.
# ============================================================================
set -uo pipefail
BIN=${ARCHON_BIN:-./target/debug/archon}
SAMPLE_N=${1:-8}
hr() { printf '\n========== %s ==========\n' "$1"; }

command -v python3 >/dev/null || { echo "python3 required"; exit 2; }
[ -x "$BIN" ] || { echo "archon binary not found at $BIN (build first)"; exit 2; }

# ---------------------------------------------------------------------------
hr "1. DOC INVENTORY"
$BIN docs list | head -3
TOTAL=$($BIN docs list | grep -cE ' doc-[0-9a-f-]+ ')
echo "... total docs: $TOTAL"
mapfile -t IDS < <($BIN docs list | grep -oE 'doc-[0-9a-f-]+' | sort -u)

# ---------------------------------------------------------------------------
hr "5. CHUNK-INTEGRITY — chunks_root (ALL docs)"
# The headline integrity proof: every ingested doc's recomputed root must match
# its sealed extract_text_spatial record. Any 'mismatch' = tamper/drift = FAIL.
$BIN docs verify-integrity
INTEG=$($BIN docs verify-integrity --json)
echo "$INTEG" | python3 -c 'import sys,json; d=json.load(sys.stdin); \
print("  ALL_PASS:", d["all_pass"], "| docs:", len(d["documents"]), \
"| no-record:", sum(1 for x in d["documents"] if x["status"]=="no-record"))'

# ---------------------------------------------------------------------------
hr "2/8. PER-DOC INSPECT (sample of $SAMPLE_N) — chunks / OCR / images / provenance"
for id in "${IDS[@]:0:$SAMPLE_N}"; do
  echo "--- $id ---"
  $BIN docs inspect "$id" 2>/dev/null | grep -iE 'chunk|ocr run|image|provenance|pages|scanned' | head -8
done

# ---------------------------------------------------------------------------
hr "7. RETRIEVAL (hybrid) — a few conceptual probes"
for q in "phenomenological evaluation of virtual learning" \
         "rhetoric and being" \
         "Umwelt and the theory of meaning"; do
  echo "--- query: $q ---"
  $BIN docs search "$q" --mode hybrid 2>/dev/null | head -6
done

# ---------------------------------------------------------------------------
hr "4/6. QUOTE VERIFY — derive a real phrase, then EXACT / FUZZY / ABSENT"
# Pull a content snippet from a top search hit and extract a clean ~8-word span.
# NOTE: the extractor lives in its own file so the piped search output owns stdin
# (a heredoc `python3 - <<EOF` would steal stdin from the pipe).
EXTRACT=$(mktemp)
cat > "$EXTRACT" <<'PY'
import sys, re
txt = sys.stdin.read()
m = re.search(r'content:\s*(.+?)(?:\n\s*\d+\.|\Z)', txt, re.S)
if not m: sys.exit(0)
words = re.sub(r'\s+', ' ', m.group(1)).strip().split(' ')
span = [w for w in words if re.search(r'[A-Za-z]', w)]  # skip headers/punct-only tokens
if len(span) >= 12:   print(' '.join(span[4:12]))
elif len(span) >= 8:  print(' '.join(span[:8]))
PY
SNIP=$($BIN docs search "evaluation" --mode hybrid --debug 2>/dev/null | python3 "$EXTRACT")
rm -f "$EXTRACT"
if [ -n "$SNIP" ]; then
  echo "derived phrase: \"$SNIP\""
  echo ">> LOCATE (expect ✓ EXACT-or-near + coord_space marker + bbox for a born-digital doc):"
  $BIN docs verify-quote "$SNIP" --json 2>/dev/null \
    | python3 -c 'import sys,json; d=json.load(sys.stdin); \
l=(d.get("locations") or [{}])[0]; f=(l.get("fragments") or [{}])[0]; \
print("   found:",d["found"],"| kind:",l.get("match_kind"),"| coord_space:",f.get("coord_space"),"| bbox:",f.get("bbox"),"| pages:",l.get("page_start"),"-",l.get("page_end"))'
  # perturb one char → fuzzy
  FUZZ=$(python3 -c "s='''$SNIP'''; i=len(s)//2; print(s[:i]+s[i+1:])")
  echo ">> FUZZY (dropped a char → expect ~ FUZZY nn%):"
  $BIN docs verify-quote "$FUZZ" 2>/dev/null | grep -E 'EXACT|FUZZY|NOT FOUND' | head -1
else
  echo "!! could not derive a phrase from search output — run verify-quote manually."
fi
echo ">> ABSENT (expect ✗ NOT FOUND):"
$BIN docs verify-quote "quantum chromodynamics lagrangian gauge invariance renormalization" 2>/dev/null \
  | grep -E 'EXACT|FUZZY|NOT FOUND' | head -1

hr "DONE — review above; every doc should be INTACT, born-digital quotes should carry marker bboxes."
