#!/usr/bin/env python3
"""M6 — live-section E2E: full FCDP stage sequence on a real pack.

D1 movement plan → G-1 → D1.5 skeleton (+counterargument) → G-1.5
→ D2 movement-by-movement (L2: no trailing-movement drops possible)
→ substitute → mechanical gauntlet → G-A → G-E + G-G → R-loop (≤3,
repair prompts restate the FULL movement plan per L2) → provenance + declaration.

usage: m6_run.py <pack.json> <workdir>
"""
import json, os, re, subprocess, sys, urllib.request
from pathlib import Path

pack_path, work = Path(sys.argv[1]).resolve(), Path(sys.argv[2])
work.mkdir(parents=True, exist_ok=True)
sb = Path(__file__).parent
pack = json.load(open(pack_path))
chain = work / "provenance.jsonl"

KEY = os.environ.get("ANTHROPIC_API_KEY")
# (fork copy: key comes from the environment only)

def fable(prompt, max_tokens=8000):
    # Fable 5: thinking is adaptive-only; effort caps runaway thinking that
    # consumed 16k output tokens with zero text on repair prompts (run1/run2).
    body = json.dumps({"model": "claude-fable-5", "max_tokens": max_tokens,
                       "thinking": {"type": "adaptive"},
                       "output_config": {"effort": "medium"},
                       "messages": [{"role": "user", "content": prompt}]}).encode()
    req = urllib.request.Request("https://api.anthropic.com/v1/messages", data=body,
        headers={"x-api-key": KEY, "anthropic-version": "2023-06-01",
                 "content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        out = json.load(r)
    text = "".join(b.get("text", "") for b in out["content"])
    if not text.strip():
        sys.exit(f"ABORT: empty model output (stop_reason={out.get('stop_reason')})")
    return text, out.get("usage", {})

def record(artifact, stage, detail):
    subprocess.run([sys.executable, str(sb / "provenance.py"), "record", str(chain.resolve()),
                    str(Path(artifact).resolve()), stage, json.dumps(detail)],
                   check=True, cwd=sb, capture_output=True)

def run(cmd):
    return subprocess.run([str(c) for c in cmd], capture_output=True, text=True, cwd=sb)

# ── shared HEAD block (P1–P3, P8, P9 + quote index + graded evidence) ──
locks = "\n".join(f"- {l}" for l in pack["p3_terminology_locks"])
neg = "\n".join(f"- {c}" for c in pack["p8_negative_constraints"])
qidx = "\n".join(f"«{q['id']}» {q['source']} {q['locus']} — {q['description']} (intended use: {q['intended_use']})"
                 for q in pack["p4a_quote_index"])
ev = "\n".join(f"[{e['id']} {e['grade']}] {e['content']}" for e in pack["p5_evidence"])
lo, hi = pack["p1_task"]["target_words"]
HEAD = f"""TASK: {pack['p1_task']['section_identity']}. Target {lo}-{hi} words total. LaTeX conventions: {pack['p1_task']['latex_conventions']}.

USAGE BOUNDARY: {pack['p9_usage_statement']}

TERMINOLOGY & STYLE LOCKS (hard constraints):
{locks}

FORBIDDEN:
{neg}

QUOTE INDEX (quotations exist ONLY as «Qnn»/«Qnn+» markers; the quoted words enter later by mechanical substitution — never write quoted words):
{qidx}

EVIDENCE BANK (grades set assertion strength — AUTHOR-CONFIRMED flat; CONFIRMED asserted with its content; UNCERTAIN hedged or omitted):
{ev}

CONCEPTUAL SEMANTICS:
{pack['p6_semantics']}

FOUNDATION TEXT (the existing prose this section must cohere with; drop none of its claims silently):
{pack['p7_foundation']}"""

RESUME = (work / "draft-presub.md").exists()

# ═══ D1 — movement plan ═══
d1_prompt = f"""{HEAD}

STAGE D1 — produce a MOVEMENT PLAN only (no prose). Output exactly this structure in markdown:

For each movement (use 3 movements):
MOVEMENT <n>: <one-sentence claim>
EVIDENCE: <ids from the evidence bank>
QUOTES: <ids from the quote index>
FOUNDATION-ANCHORS: <which foundation claims this movement carries forward>
WORD-SHARE: <target words>
STYLE-NOTE: <sentence-rhythm intent for this movement, one line>

Then:
FOUNDATION DISPOSITION: for each distinct claim in the foundation text — RETAIN / EXPAND / CORRECT(state it) / OMIT(reason).
LEDGER: every quote ID and evidence ID — ASSIGNED(movement) or UNUSED(reason)."""
if RESUME:
    print("── D1 (resumed from disk)")
    d1 = (work / "d1-plan.md").read_text()
else:
    print("── D1 movement plan …")
    d1, u = fable(d1_prompt)
    (work / "d1-plan.md").write_text(d1)
    record(work / "d1-plan.md", "d1-plan", {"usage": u, "gates_run": []})

# G-1: ledger closed, all IDs assigned or reasoned
missing = [q["id"] for q in pack["p4a_quote_index"] if q["id"] not in d1] + \
          [e["id"] for e in pack["p5_evidence"] if e["id"] not in d1]
g1_pass = not missing and "MOVEMENT 3" in d1.upper().replace("**", "")
print(f"   G-1: {'PASS' if g1_pass else f'FAIL missing={missing}'}")
if not g1_pass:
    sys.exit("G-1 failed — plan incomplete; rerun D1")
if not RESUME:
    record(work / "d1-plan.md", "g1-gate", {"pass": True, "gates_run": ["G-1"]})

# ═══ D1.5 — skeleton ═══
d15_prompt = f"""{HEAD}

APPROVED MOVEMENT PLAN:
{d1}

STAGE D1.5 — produce a SKELETON only (structure, no prose). For each movement:
NUCLEUS claims in order; under each, its SATELLITES labeled evidence/elaboration/concession/contrast/restatement, with each assigned quote ID and evidence ID attached to the satellite it serves.
COUNTERARGUMENT: for each major nucleus claim, the strongest objection an actual interlocutor of this dissertation could press (from the pack only), with disposition ANSWER(which satellite) / CONCEDE-AND-LIMIT / SURFACE-TO-USER.
TRANSITIONS: the discourse relation each movement boundary performs.
RHYTHM: where short landing sentences fall; where long periodic builds run."""
if RESUME:
    print("── D1.5 (resumed from disk)")
    d15 = (work / "d15-skeleton.md").read_text()
    surface = "SURFACE-TO-USER" in d15
else:
    print("── D1.5 skeleton …")
    d15, u = fable(d15_prompt)
    (work / "d15-skeleton.md").write_text(d15)
    surface = "SURFACE-TO-USER" in d15
    record(work / "d15-skeleton.md", "d15-skeleton",
           {"usage": u, "surface_to_user": surface, "gates_run": ["G-1.5"]})
print(f"   G-1.5: skeleton produced{' — SURFACE-TO-USER item present (reported at handoff)' if surface else ''}")

# ═══ D2 — movement by movement (L2: trailing drops structurally impossible) ═══
movements = re.split(r"(?=MOVEMENT\s+\d)", d1)
movements = [m for m in movements if re.match(r"MOVEMENT\s+\d", m.strip())][:3]
ex_by_type = {}
for e in pack.get("p2b_exemplars", []):
    ex_by_type.setdefault(e["movement_type"], []).append(e["text"])
mv_types = ["theoretical-exposition", "theoretical-exposition", "transition-argument"]
parts = []
for i, mv in enumerate(movements):
    if RESUME:
        parts.append((work / f"d2-m{i+1}.md").read_text().strip())
        continue
    exs = "\n\n".join(ex_by_type.get(mv_types[i], [])[:2])
    prior = ("\n\nALREADY-DRAFTED PRECEDING MOVEMENTS (continue from them; do not repeat them):\n"
             + "\n\n".join(parts)) if parts else ""
    d2_prompt = f"""{HEAD}

FULL MOVEMENT PLAN (for orientation — you are drafting ONLY movement {i+1} now):
{d1}

SKELETON FOR THIS MOVEMENT (follow its satellite structure and rhythm placements):
{movements[i]}

{d15}

VOICE EXEMPLARS for this movement type (match their texture; do NOT quote or closely paraphrase them — any shared 8-word sequence is a gate failure):
{exs}{prior}

STAGE D2 — write ONLY movement {i+1} as finished prose. Quotations ONLY as «Qnn+» or «Qnn» markers placed where the quote belongs, with your prose written around them. Output only the movement's prose body."""
    print(f"── D2 movement {i+1}/3 …")
    p, u = fable(d2_prompt)
    parts.append(p.strip())
    record_detail = {"movement": i + 1, "usage": u, "gates_run": []}
    (work / f"d2-m{i+1}.md").write_text(p)
    record(work / f"d2-m{i+1}.md", f"d2-m{i+1}", record_detail)

draft = "\n\n".join(parts)
if not RESUME:
    (work / "draft-presub.md").write_text(draft)
    record(work / "draft-presub.md", "d2-assembled", {"words": len(draft.split()), "gates_run": []})
print(f"   assembled: {len(draft.split())} words{' (resumed)' if RESUME else ''}")

# ═══ Gauntlet + R-loop ═══
def full_gauntlet(tag):
    res = {}
    r = run([os.environ.get("FCDP_BIN", str(sb.parents[1] / "target/debug/archon-fcdp")), "substitute", work / f"draft-presub{tag}.md",
             pack_path, work / f"draft{tag}.md"])
    res["sub"] = r.returncode == 0
    r = run([sys.executable, sb / "gauntlet.py", work / f"draft{tag}.md", pack_path,
             "--presub", work / f"draft-presub{tag}.md"])
    res["mech"] = json.loads(r.stdout)
    r = run([os.environ.get("FCDP_BIN", str(sb.parents[1] / "target/debug/archon-fcdp")), "ga-gate", work / f"draft{tag}.md",
             sb.parents[1] / "crates/archon-draft/data/ga-gate-locked-v2.json"])
    res["ga"] = json.loads(r.stdout)
    (work / f"ga-report{tag}.json").write_text(r.stdout)
    r = run([sys.executable, sb / "judge.py", "GE", work / f"draft{tag}.md", pack_path])
    res["ge"] = json.loads(r.stdout)
    r = run([sys.executable, sb / "judge.py", "GG", work / f"draft{tag}.md", pack_path,
             "--plan", work / "d1-plan.md"])
    res["gg"] = json.loads(r.stdout)
    (work / f"gauntlet-full{tag}.json").write_text(json.dumps(
        {k: (v if isinstance(v, bool) else {kk: v[kk] for kk in v if kk != "transcript"})
         for k, v in res.items()}, indent=1))
    record(work / f"gauntlet-full{tag}.json", f"gauntlet{tag or '-0'}",
           {"sub": res["sub"], "mech": res["mech"]["pass"], "ga": res["ga"]["pass"],
            "ge": res["ge"]["pass"], "gg": res["gg"]["pass"],
            "gates_run": ["G-A", "G-B", "G-C", "G-D", "G-E", "G-F", "G-G"]})
    res["all"] = res["sub"] and res["mech"]["pass"] and res["ga"]["pass"] and res["ge"]["pass"] and res["gg"]["pass"]
    return res

def named_defects(res):
    out = list(res["mech"]["defects"]) if not res["mech"]["pass"] else []
    if not res["ga"]["pass"]:
        out += res["ga"]["hard_failures"] + res["ga"]["label_failures"]
    if not res["ge"]["pass"]:
        out += res["ge"]["defects"]
    if not res["gg"]["pass"]:
        out += res["gg"]["defects"]
    return out

tag = ""
print("── full gauntlet (cycle 0) …")
res = full_gauntlet(tag)
print(f"   sub={res['sub']} mech={res['mech']['pass']} ga={res['ga']['pass']} ge={res['ge']['pass']} gg={res['gg']['pass']}")

cycle = 0
while not res["all"] and cycle < 3:
    cycle += 1
    defs = named_defects(res)
    print(f"── R-cycle {cycle}: {len(defs)} named defect(s)")
    for d in defs[:6]:
        print(f"     · {d[:110]}")
    exs = "\n\n".join(t for ts in ex_by_type.values() for t in ts[:1])
    rep_prompt = f"""Revise the passage below to fix ONLY the named defects — nothing else. Where a defect names missing content, restore exactly that; where it names unwarranted content, remove exactly that; where it names a style metric, adjust sentence architecture only, landing INSIDE the band, mid-band, not past it. Never add, drop, or alter any other claim, «Qnn» marker, or evidence assertion.

THE PASSAGE MUST REALIZE THIS COMPLETE MOVEMENT PLAN — every movement, every assigned quote marker, every evidence commitment (a repair that drops any of these is a failed repair):
{d1}

NAMED DEFECTS:
{chr(10).join('- ' + d for d in defs)}

VOICE EXEMPLARS (match texture; never quote or closely paraphrase; no shared 8-word sequence):
{exs}

PASSAGE:
{(work / f"draft-presub{tag}.md").read_text()}

Output ONLY the revised passage body, quotations only as «Qnn»/«Qnn+» markers."""
    rev, u = fable(rep_prompt, max_tokens=16000)
    tag = f"-r{cycle}"
    (work / f"draft-presub{tag}.md").write_text(rev)
    record(work / f"draft-presub{tag}.md", "revision",
           {"cycle": cycle, "defects_addressed": len(defs), "usage": u, "gates_run": []})
    print(f"── full re-gauntlet (cycle {cycle}) …")
    res = full_gauntlet(tag)
    print(f"   sub={res['sub']} mech={res['mech']['pass']} ga={res['ga']['pass']} ge={res['ge']['pass']} gg={res['gg']['pass']}")

# ═══ outcome ═══
status = "ALL GATES GREEN" if res["all"] else f"STOP-AND-SURFACE after {cycle} cycle(s)"
print(f"\n═══ M6 outcome: {status} ═══")
if not res["all"]:
    print("surfaced defects:")
    for d in named_defects(res):
        print(f"  · {d[:140]}")
r = run([sys.executable, sb / "provenance.py", "verify", chain])
print(r.stdout.strip())
r = run([sys.executable, sb / "provenance.py", "declare", chain, pack_path])
(work / "declaration.md").write_text(r.stdout)
final = (work / f"draft{tag}.md")
print(f"final draft: {final} ({len(final.read_text().split())} words)")
json.dump({"status": status, "cycles": cycle, "final_draft": str(final),
           "surface_to_user_in_skeleton": surface},
          open(work / "m6-outcome.json", "w"), indent=1)
