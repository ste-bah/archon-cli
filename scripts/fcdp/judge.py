#!/usr/bin/env python3
"""FCDP sandbox — judge gates G-E (foundation fidelity) and G-G (consistency).

Move-4/5 hardening:
  * fresh lean-context call (draft + rubric + named pack fields ONLY — no session history)
  * rationale-BEFORE-verdict, per battery item
  * battery order randomized with a deterministic seed derived from section_id (recorded)
  * defect-affinity blinding: the prompt never says which stage produced the text
    or that it already passed other gates
  * free-prose output; verdicts extracted mechanically; anything not cleanly
    YES/NO fails closed (counts toward defect side)

usage: judge.py <gate: GE|GG> <draft-file> <pack.json> [--model claude-fable-5]
"""
import hashlib, json, os, re, sys, urllib.request

gate, draft_path, pack_path = sys.argv[1], sys.argv[2], sys.argv[3]
model = sys.argv[sys.argv.index("--model") + 1] if "--model" in sys.argv else "claude-fable-5"
plan = open(sys.argv[sys.argv.index("--plan") + 1]).read() if "--plan" in sys.argv else None

draft = open(draft_path).read()
pack = json.load(open(pack_path))

KEY = os.environ.get("ANTHROPIC_API_KEY")
# (fork copy: key comes from the environment only)
assert KEY, "no ANTHROPIC_API_KEY"

# ── Batteries (binary items; YES = defect on 2/4-type items is handled per-item `bad_on`) ──
if gate == "GE":
    ctx = (f"CONCEPTUAL SEMANTICS (assertions warranted here are in-pack):\n{pack['p6_semantics']}\n\n"
           f"FOUNDATION TEXT:\n{pack['p7_foundation']}\n\n"
           f"EVIDENCE BANK (grades set assertion strength):\n"
           + "\n".join(f"[{e['id']} {e['grade']}] {e['content']}" for e in pack["p5_evidence"])
           + "\n\nQUOTE INDEX (intended rhetorical use):\n"
           + "\n".join(f"[{q['id']}] {q['source']} {q['locus']} — intended: {q['intended_use']}"
                       for q in pack["p4a_quote_index"]))
    battery = [
        ("E1", "Is every claim of the foundation text present in the passage (none silently dropped)?", "NO"),
        ("E2", "Does the passage alter any foundation claim's strength or modality beyond what its evidence grade licenses (e.g., asserting an UNCERTAIN item flatly)?", "YES"),
        ("E3", "Does the passage assert anything with no warrant in the evidence bank or foundation text?", "YES"),
        ("E4", "Is each quotation used in a way consistent with its stated intended rhetorical use in the quote index?", "NO"),
        ("E5", "Are all evidence items asserted at a strength matching their grade (AUTHOR-CONFIRMED flat; CONFIRMED with its measure; UNCERTAIN hedged or omitted)?", "NO"),
    ]
elif gate == "GG":
    ctx = ("TERMINOLOGY LOCKS:\n" + "\n".join(pack["p3_terminology_locks"])
           + f"\n\nCONCEPTUAL SEMANTICS:\n{pack['p6_semantics']}"
           + (f"\n\nAPPROVED MOVEMENT PLAN (the claims the passage is committed to; FCDP G-G rubric context):\n{plan}" if plan else ""))
    battery = [
        ("G1", "Is every locked term used per its locked definition at every occurrence?", "NO"),
        ("G2", "Does any later passage contradict an earlier passage's claim without explicit acknowledgment?", "YES"),
        ("G3", "Is each evidence item characterized with the same value/strength everywhere it appears?", "NO"),
        ("G4", "Do any two passages characterize the same concept incompatibly?", "YES"),
    ]
else:
    sys.exit("gate must be GE or GG")

# deterministic battery order randomization, seed recorded
seed = int(hashlib.sha256(pack["meta"]["section_id"].encode()).hexdigest()[:8], 16)
order = list(range(len(battery)))
r = seed
for i in range(len(order) - 1, 0, -1):  # LCG Fisher-Yates (no random module → reproducible everywhere)
    r = (r * 1103515245 + 12345) % (2**31)
    j = r % (i + 1)
    order[i], order[j] = order[j], order[i]
battery = [battery[i] for i in order]

items = "\n".join(f"{i+1}. [{b[0]}] {b[1]}" for i, b in enumerate(battery))
prompt = f"""You are evaluating a passage of scholarly prose against reference material. Answer each numbered question independently.

REFERENCE MATERIAL:
{ctx}

PASSAGE UNDER EVALUATION:
{draft}

QUESTIONS:
{items}

For EACH question, in order: give one sentence of reasoning first, THEN end the line with exactly "VERDICT: YES" or "VERDICT: NO". Answer every question. Do not summarize at the end."""

body = json.dumps({"model": model, "max_tokens": 9000,
                   "thinking": {"type": "adaptive"},
                   "output_config": {"effort": "medium"},
                   "messages": [{"role": "user", "content": prompt}]}).encode()
req = urllib.request.Request("https://api.anthropic.com/v1/messages", data=body,
    headers={"x-api-key": KEY, "anthropic-version": "2023-06-01", "content-type": "application/json"})
with urllib.request.urlopen(req, timeout=180) as resp:
    out = json.load(resp)
text = "".join(b.get("text", "") for b in out["content"])

# ── extraction: block-based (item N start → item N+1 start); fail-closed ──
defects, transcript = [], []
starts = []
for i in range(len(battery)):
    m = re.search(rf"^\s*(?:\*\*)?{i+1}[\.\)]", text, flags=re.M)
    starts.append(m.start() if m else None)
for i, (code, q, bad_on) in enumerate(battery):
    if starts[i] is None:
        block = ""
    else:
        nxt = next((s for s in starts[i+1:] if s is not None), len(text))
        block = text[starts[i]:nxt]
    verdict = None
    m = re.search(r"VERDICT:?\s*\**\s*(YES|NO)", block, flags=re.I)
    if m:
        verdict = m.group(1).upper()
    match = " ".join(block.split())[:300] if block else None
    transcript.append({"item": code, "question": q, "line": match or "(not found)", "verdict": verdict})
    if verdict is None:
        defects.append(f"{gate}/{code}: no clean verdict extracted — fail-closed")
    elif verdict == bad_on:
        defects.append(f"{gate}/{code}: {match.strip()}")

report = {"gate": gate, "model": model, "stop_reason": out.get("stop_reason"),
          "battery_seed": seed,
          "battery_order": [b[0] for b in battery],
          "defects": defects, "pass": not defects, "transcript": transcript,
          "usage": out.get("usage", {})}
print(json.dumps(report, indent=1))
sys.exit(0 if report["pass"] else 1)
