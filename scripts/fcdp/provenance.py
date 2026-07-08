#!/usr/bin/env python3
"""FCDP sandbox — provenance chain (Move 7) + disclosure declaration (Move 6).

Records are shape-compatible with archon-provenance's ProvenanceRecord/Edge
(record_id, artifact_id, parent hash chain); stored as JSONL here, promoted to
the CozoDB store at M7 integration.

usage:
  provenance.py record <chain.jsonl> <artifact-file> <stage> <detail-json>
  provenance.py verify <chain.jsonl>
  provenance.py declare <chain.jsonl> <pack.json>       → disclosure declaration (md)
"""
import hashlib, json, sys
from pathlib import Path

def sha(s): return hashlib.sha256(s.encode() if isinstance(s, str) else s).hexdigest()

cmd, chain_path = sys.argv[1], Path(sys.argv[2])

if cmd == "record":
    artifact, stage, detail = Path(sys.argv[3]), sys.argv[4], json.loads(sys.argv[5])
    prev = "GENESIS"
    if chain_path.exists():
        lines = chain_path.read_text().strip().splitlines()
        if lines:
            prev = json.loads(lines[-1])["chain_hash"]
    content_hash = sha(artifact.read_bytes())
    rec = {"record_id": f"{stage}-{content_hash[:12]}", "artifact_id": str(artifact),
           "stage": stage, "content_sha256": content_hash, "detail": detail,
           "prev_chain_hash": prev}
    rec["chain_hash"] = sha(prev + content_hash + json.dumps(detail, sort_keys=True))
    with open(chain_path, "a") as f:
        f.write(json.dumps(rec) + "\n")
    print(f"recorded {rec['record_id']} (chain {rec['chain_hash'][:12]}…)")

elif cmd == "verify":
    prev = "GENESIS"
    ok = True
    for i, line in enumerate(chain_path.read_text().strip().splitlines()):
        rec = json.loads(line)
        want = sha(prev + rec["content_sha256"] + json.dumps(rec["detail"], sort_keys=True))
        if rec["prev_chain_hash"] != prev or rec["chain_hash"] != want:
            print(f"CHAIN BREAK at record {i}: {rec['record_id']}")
            ok = False
        prev = rec["chain_hash"]
    print("chain VERIFIED" if ok else "chain INVALID")
    sys.exit(0 if ok else 1)

elif cmd == "declare":
    pack = json.load(open(sys.argv[3]))
    recs = [json.loads(l) for l in chain_path.read_text().strip().splitlines()]
    stages = [r["stage"] for r in recs]
    cycles = sum(1 for s in stages if s == "revision")
    gates = sorted({g for r in recs for g in r["detail"].get("gates_run", [])})
    dec = f"""## Declaration of AI use — {pack['meta']['section_id']}

*(Draft for author revision — generated from the provenance chain; formatted to the
Cambridge template-declaration structure at HGSE tool/prompt/integration granularity.)*

**Tools.** Anthropic Claude (Fable 5, `claude-fable-5`) for drafting and judge-gate
evaluation, orchestrated by the FCDP v2 protocol; Lanham stylometric analyzer
(`archon-lanham`, Rust) for mechanical style measurement.

**What the model contributed.** Phrasing, discourse structure, and analysis of
evidence supplied in a sealed, pre-verified context pack ({len(pack['p4a_quote_index'])}
quotations, {len(pack['p5_evidence'])} graded evidence items). Stage sequence recorded:
{' → '.join(dict.fromkeys(stages))}.

**What the model was barred from.** Introducing claims beyond the evidence bank;
inventing or recalling citations (all loci trace to the pack; violations gate-fail);
generating quoted text (quotations enter only by mechanical ID substitution from a
PDF-verified bank). No retrieval was available to the model during drafting.

**Human control points.** Plan approval before drafting; gate reports at every cycle
({cycles} revision cycle(s); gates run: {', '.join(gates) or 'n/a'}); final acceptance
by the author. Nothing entered the dissertation without author sign-off.

**Verification trail.** {len(recs)} hash-chained provenance records (chain head
`{recs[-1]['chain_hash'][:16]}…`), exportable to W3C-PROV. Full chain retained with
the dissertation sources.
"""
    print(dec)
