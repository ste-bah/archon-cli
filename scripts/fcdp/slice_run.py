#!/usr/bin/env python3
"""FCDP sandbox — D2-slice checkpoint driver (M3 orchestrator MVP + M5 loop).

pack → D2 (per-movement prompt, «Qnn» markers, exemplars+rhythm directives)
     → substitute (Rust fcdp) → mechanical gauntlet → G-A (Rust, section scale)
     → G-E judge → provenance record per step → chain verify → declaration.

Single revision cycle max in the slice (full ≤3 loop logic identical in shape).
usage: slice_run.py <pack.json> <workdir>
"""
import json, os, subprocess, sys, urllib.request
from pathlib import Path

pack_path, work = Path(sys.argv[1]).resolve(), Path(sys.argv[2])
work.mkdir(parents=True, exist_ok=True)
sb = Path(__file__).parent
pack = json.load(open(pack_path))
chain = work / "provenance.jsonl"
if chain.exists():
    chain.unlink()

KEY = os.environ.get("ANTHROPIC_API_KEY")
# (fork copy: key comes from the environment only)

def fable(prompt, max_tokens=6000):
    body = json.dumps({"model": "claude-fable-5", "max_tokens": max_tokens,
                       "messages": [{"role": "user", "content": prompt}]}).encode()
    req = urllib.request.Request("https://api.anthropic.com/v1/messages", data=body,
        headers={"x-api-key": KEY, "anthropic-version": "2023-06-01", "content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=240) as r:
        out = json.load(r)
    text = "".join(b.get("text", "") for b in out["content"])
    if not text.strip():
        sys.exit(f"ABORT: empty model output (stop_reason={out.get('stop_reason')})")
    return text, out.get("usage", {})

def record(artifact, stage, detail):
    subprocess.run([sys.executable, sb / "provenance.py", "record", chain, artifact, stage,
                    json.dumps(detail)], check=True, cwd=sb)

def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, cwd=sb, **kw)

# ── D2 prompt (mini movement plan inlined — D1/D1.5 product for the slice) ──
locks = "\n".join(f"- {l}" for l in pack["p3_terminology_locks"])
neg = "\n".join(f"- {c}" for c in pack["p8_negative_constraints"])
qidx = "\n".join(f"«{q['id']}» {q['source']} {q['locus']} — {q['description']} (use as: {q['intended_use']})"
                 for q in pack["p4a_quote_index"])
ev = "\n".join(f"[{e['id']} {e['grade']}] {e['content']}" for e in pack["p5_evidence"])
lo, hi = pack["p1_task"]["target_words"]
d2 = f"""You are drafting a short passage of dissertation prose. Follow every constraint exactly.

TASK: {pack['p1_task']['section_identity']}. Target {lo}-{hi} words. LaTeX conventions: {pack['p1_task']['latex_conventions']}.

USAGE BOUNDARY: {pack['p9_usage_statement']}

TERMINOLOGY & STYLE LOCKS:
{locks}

FORBIDDEN:
{neg}

MOVEMENT PLAN (draft both movements, in order):
M1 — claim: the soul is a capacity-form, not an identity; the first actuality is a capacity for the second. Evidence: E1. Quote: «Q1» (emit the marker «Q1+» where the quote belongs — NEVER write the quoted words yourself). Rhythm: longer periodic builds, one short landing sentence.
M2 — claim: affectability grounds the possibility of discourse. Evidence: E2 (assert with its content), E3 (UNCERTAIN — hedge or omit). Quote: «Q2+» (marker only). Rhythm: run one long subordinated sentence into a compact close.

QUOTE INDEX (markers you may use — the quoted words enter later by mechanical substitution):
{qidx}

EVIDENCE BANK (grades set assertion strength — AUTHOR-CONFIRMED flat, CONFIRMED with its content, UNCERTAIN hedged/omitted):
{ev}

FOUNDATION (build on this, drop nothing silently):
{pack['p7_foundation']}

CONCEPTUAL SEMANTICS: {pack['p6_semantics']}

Write the passage now. Output ONLY the passage body (no title, no commentary). Quotations ONLY as «Qnn+» markers."""

print("── D2 drafting call …")
draft, usage = fable(d2)
(work / "draft-presub.md").write_text(draft)
record(work / "draft-presub.md", "d2-draft", {"model": "claude-fable-5", "usage": usage,
                                              "gates_run": []})
print(f"   {len(draft.split())} words, usage {usage.get('input_tokens')}in/{usage.get('output_tokens')}out")

print("── substitution (Rust fcdp) …")
r = run([str(os.environ.get("FCDP_BIN", str(sb.parents[1] / "target/debug/archon-fcdp"))), "substitute", str(work / "draft-presub.md"),
         str(pack_path), str(work / "draft.md")])
print("  ", (r.stdout + r.stderr).strip().replace("\n", " | "))
sub_ok = r.returncode == 0
record(work / "draft.md", "substitute", {"exit": r.returncode, "gates_run": ["G-B/sub"]})

print("── mechanical gauntlet …")
r = run([sys.executable, str(sb / "gauntlet.py"), str(work / "draft.md"), str(pack_path),
         "--presub", str(work / "draft-presub.md")])
mech = json.loads(r.stdout)
(work / "gauntlet-report.json").write_text(r.stdout)
record(work / "gauntlet-report.json", "gauntlet", {"pass": mech["pass"],
       "defects": len(mech["defects"]), "gates_run": ["G-B", "G-C", "G-D", "G-F"]})
print(f"   pass={mech['pass']} defects={mech['defects']} advisories={len(mech['advisories'])}")

print("── G-A (Rust, section scale) …")
r = run([str(os.environ.get("FCDP_BIN", str(sb.parents[1] / "target/debug/archon-fcdp"))), "ga-gate", str(work / "draft.md"),
         str(sb.parents[1] / "crates/archon-draft/data/ga-gate-locked-v2.json")])
ga = json.loads(r.stdout)
(work / "ga-report.json").write_text(r.stdout)
record(work / "ga-report.json", "ga-gate", {"pass": ga["pass"], "hard": ga["hard_failures"],
       "labels": ga["label_failures"], "gates_run": ["G-A"]})
print(f"   pass={ga['pass']} hard={ga['hard_failures']} labels={ga['label_failures']}")
print(f"   advisories: {len(ga['advisories'])} (section scale)")

print("── G-E judge (fresh lean context) …")
r = run([sys.executable, str(sb / "judge.py"), "GE", str(work / "draft.md"), str(pack_path)])
ge = json.loads(r.stdout)
(work / "ge-report.json").write_text(r.stdout)
record(work / "ge-report.json", "judge-GE", {"pass": ge["pass"], "defects": ge["defects"],
       "seed": ge["battery_seed"], "gates_run": ["G-E"]})
print(f"   pass={ge['pass']}")
for d in ge["defects"]:
    print(f"   DEFECT {d[:120]}")

all_pass = sub_ok and mech["pass"] and ga["pass"] and ge["pass"]
print(f"\n── slice verdict: {'ALL GATES GREEN' if all_pass else 'defects found (R-cycle would follow — expected behavior)'}")

print("── provenance chain verify …")
r = run([sys.executable, str(sb / "provenance.py"), "verify", str(chain)])
print("  ", r.stdout.strip())

print("── disclosure declaration …")
r = run([sys.executable, str(sb / "provenance.py"), "declare", str(chain), str(pack_path)])
(work / "declaration.md").write_text(r.stdout)
print("   written to", work / "declaration.md")
