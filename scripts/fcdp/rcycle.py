#!/usr/bin/env python3
"""FCDP sandbox — one targeted R-cycle for a G-A failure (repair rule: sentence
architecture ONLY — claims, quotes, evidence untouched), then full re-gauntlet.
usage: rcycle.py <workdir> <pack.json>
"""
import json, os, subprocess, sys, urllib.request
from pathlib import Path

work, pack_path = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
cycle = int(sys.argv[3]) if len(sys.argv) > 3 else 1
extra_defects = sys.argv[4].split("||") if len(sys.argv) > 4 else []
sb = Path(__file__).parent
pack = json.load(open(pack_path))
prev_suffix = "" if cycle == 1 else f"-r{cycle-1}"
ga = json.load(open(work / f"ga-report{prev_suffix}.json"))
draft = (work / f"draft-presub{prev_suffix}.md").read_text()  # revise PRE-substitution (markers intact)
chain = work / "provenance.jsonl"

KEY = os.environ.get("ANTHROPIC_API_KEY")
# (fork copy: key comes from the environment only)

defect_lines = "\n".join(f"- {d}" for d in ga["hard_failures"] + ga["label_failures"] + extra_defects)
exemplars = "\n\n".join(f"[{e['movement_type']}]\n{e['text']}" for e in pack.get("p2b_exemplars", []))
ex_block = f"\n\nVOICE EXEMPLARS (match the texture of these passages by the same author — do NOT quote or closely paraphrase them; any shared 8-word sequence is a gate failure):\n{exemplars}" if exemplars else ""
prompt = f"""Revise the passage below to fix ONLY the named defects — nothing else. Where a defect names missing content, restore exactly that content; where a defect names unwarranted content, remove exactly that; where a defect names a style metric, adjust sentence architecture only. Never add, drop, or alter any other claim, «Qnn» marker, or evidence assertion.

NAMED DEFECTS:
{defect_lines}{ex_block}

REPAIR GUIDANCE: move each metric INTO its band, not past it — bands are ranges, land mid-band. Average sentence length must settle between 27 and 44 words. The prose should be dense and subordinated but not maximally so; voiced means concrete subjects acting, some first-person-of-the-argument presence, dynamics that rise and settle. The passage MUST retain every movement, every claim, and every «Qnn» marker from the original — a repair that drops content is a failed repair. Keep the register scholarly, the LaTeX conventions, and all terminology locks intact.

PASSAGE:
{draft}

Output ONLY the revised passage body."""

body = json.dumps({"model": "claude-fable-5", "max_tokens": 6000,
                   "messages": [{"role": "user", "content": prompt}]}).encode()
req = urllib.request.Request("https://api.anthropic.com/v1/messages", data=body,
    headers={"x-api-key": KEY, "anthropic-version": "2023-06-01", "content-type": "application/json"})
with urllib.request.urlopen(req, timeout=240) as r:
    out = json.load(r)
rev = "".join(b.get("text", "") for b in out["content"])
if not rev.strip():
    sys.exit(f"ABORT: empty revision (stop_reason={out.get('stop_reason')}, usage={out.get('usage',{}).get('output_tokens_details')}) — never gate an empty draft")
(work / f"draft-presub-r{cycle}.md").write_text(rev)

def run(cmd):
    return subprocess.run([str(c) for c in cmd], capture_output=True, text=True, cwd=sb)

def record(artifact, stage, detail):
    run([sys.executable, sb / "provenance.py", "record", chain.resolve(), Path(artifact).resolve(),
         stage, json.dumps(detail)])

record(work / f"draft-presub-r{cycle}.md", "revision",
       {"cycle": cycle, "triggered_by": ga["hard_failures"] + ga["label_failures"],
        "rule": "sentence-architecture-only", "gates_run": []})

# full re-gauntlet
r = run([os.environ.get("FCDP_BIN", str(sb.parents[1] / "target/debug/archon-fcdp")), "substitute", work / f"draft-presub-r{cycle}.md", pack_path, work / f"draft-r{cycle}.md"])
sub_ok = r.returncode == 0
r = run([sys.executable, sb / "gauntlet.py", work / f"draft-r{cycle}.md", pack_path, "--presub", work / f"draft-presub-r{cycle}.md"])
mech = json.loads(r.stdout)
r = run([os.environ.get("FCDP_BIN", str(sb.parents[1] / "target/debug/archon-fcdp")), "ga-gate", work / f"draft-r{cycle}.md", sb.parents[1] / "crates/archon-draft/data/ga-gate-locked-v2.json"])
ga2 = json.loads(r.stdout)
(work / f"ga-report-r{cycle}.json").write_text(r.stdout)
r = run([sys.executable, sb / "judge.py", "GE", work / f"draft-r{cycle}.md", pack_path])
ge = json.loads(r.stdout)
record(work / f"ga-report-r{cycle}.json", "regauntlet",
       {"cycle": cycle, "sub": sub_ok, "mech_pass": mech["pass"], "ga_pass": ga2["pass"],
        "ga_hard": ga2["hard_failures"], "ga_labels": ga2["label_failures"], "ge_pass": ge["pass"],
        "gates_run": ["G-A", "G-B", "G-C", "G-D", "G-E", "G-F"]})

print(json.dumps({"cycle": cycle, "substitution": sub_ok, "mechanical": mech["pass"],
                  "ga": {"pass": ga2["pass"], "hard": ga2["hard_failures"], "labels": ga2["label_failures"]},
                  "ge": ge["pass"],
                  "all_green": sub_ok and mech["pass"] and ga2["pass"] and ge["pass"]}, indent=1))
