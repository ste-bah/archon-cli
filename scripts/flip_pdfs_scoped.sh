#!/usr/bin/env bash
# Scoped PDF-only corpus flip (2026-07-02). Ingests ONE PDF at a time from a
# file list, so the directory walker never picks up non-PDF artifacts.
# Usage: bash scripts/flip_pdfs_scoped.sh <abs-pdf-list> <logfile>
set -uo pipefail
LIST="${1:?need pdf list}"
LOG="${2:?need logfile}"
BIN=./target/debug/archon
export PATH="$HOME/.cargo/bin:$PATH" LIBCLANG_PATH=/usr/lib/llvm-18/lib

TOTAL=$(wc -l < "$LIST")
i=0
echo "scoped flip start: $(date) — $TOTAL PDFs" | tee "$LOG"
while IFS= read -r pdf; do
  [ -z "$pdf" ] && continue
  i=$((i+1))
  echo "===== [$i/$TOTAL] $(date +%H:%M:%S) :: $pdf" >> "$LOG"
  if "$BIN" docs ingest "$pdf" --yes >> "$LOG" 2>&1; then
    echo "----- [$i/$TOTAL] OK" >> "$LOG"
  else
    rc=$?
    echo "----- [$i/$TOTAL] FAILED rc=$rc :: $pdf" | tee -a "$LOG"
  fi
done < "$LIST"
echo "scoped flip done: $(date) — processed $i/$TOTAL" | tee -a "$LOG"
