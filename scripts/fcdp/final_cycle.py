#!/usr/bin/env python3
"""M6 final repair cycle (clean instruments): fix the single named G-A defect
on draft-r3 (sentence-architecture only), then full re-gauntlet + provenance.
usage: final_cycle.py <workdir> <pack.json>
"""
import json, os, subprocess, sys, urllib.request
from pathlib import Path

work, pack_path = Path(sys.argv[1]), Path(sys.argv[2]).resolve()
sb = Path(__file__).parent
pack = json.load(open(pack_path))
chain = work / "provenance.jsonl"
d1 = (work / "d1-plan.md").read_text()
draft = (work / "draft-presub-r3.md").read_text()

KEY = os.environ.get("ANTHROPIC_API_KEY")
# (fork copy: key comes from the environment only)

prompt = f"""Revise the passage below to fix ONE named style defect — nothing else. This is a sentence-architecture repair: never add, drop, or alter any claim, «Qnn» marker, or evidence assertion, and change as few sentences as possible.

NAMED DEFECT:
- average sentence length is 44.9 words; the ceiling is 44.8. Split ONE or TWO of the longest sentences at a natural clause boundary so the average lands near 40 words. Do not shorten the passage; do not simplify diction; every other sentence stays exactly as written.

THE PASSAGE MUST CONTINUE TO REALIZE THIS COMPLETE MOVEMENT PLAN (all movements, all markers, all evidence commitments):
{d1}

PASSAGE:
{draft}

Output ONLY the revised passage body, quotations only as «Qnn»/«Qnn+» markers."""

body = json.dumps({"model": "claude-fable-5", "max_tokens": 10000,
                   "thinking": {"type": "adaptive"}, "output_config": {"effort": "medium"},
                   "messages": [{"role": "user", "content": prompt}]}).encode()
req = urllib.request.Request("https://api.anthropic.com/v1/messages", data=body,
    headers={"x-api-key": KEY, "anthropic-version": "2023-06-01", "content-type": "application/json"})
with urllib.request.urlopen(req, timeout=300) as r:
    out = json.load(r)
rev = "".join(b.get("text", "") for b in out["content"])
assert rev.strip(), f"empty output ({out.get('stop_reason')})"
(work / "draft-presub-r4.md").write_text(rev)

def run(cmd):
    return subprocess.run([str(c) for c in cmd], capture_output=True, text=True, cwd=sb)
def record(artifact, stage, detail):
    run([sys.executable, sb / "provenance.py", "record", chain.resolve(),
         Path(artifact).resolve(), stage, json.dumps(detail)])

record(work / "draft-presub-r4.md", "revision",
       {"cycle": 4, "note": "clean-instrument cycle after gate/judge instrument fixes; single G-A epsilon defect",
        "triggered_by": ["T1 avg_sentence_length 44.867 > 44.756"], "gates_run": []})

# full gauntlet
res = {}
r = run([os.environ.get("FCDP_BIN", str(sb.parents[1] / "target/debug/archon-fcdp")), "substitute", work / "draft-presub-r4.md", pack_path, work / "draft-r4.md"])
res["sub"] = r.returncode == 0
r = run([sys.executable, sb / "gauntlet.py", work / "draft-r4.md", pack_path, "--presub", work / "draft-presub-r4.md"])
mech = json.loads(r.stdout); res["mech"] = mech["pass"]
r = run([os.environ.get("FCDP_BIN", str(sb.parents[1] / "target/debug/archon-fcdp")), "ga-gate", work / "draft-r4.md", sb.parents[1] / "crates/archon-draft/data/ga-gate-locked-v2.json"])
ga = json.loads(r.stdout); res["ga"] = ga["pass"]
(work / "ga-report-r4.json").write_text(r.stdout)
r = run([sys.executable, sb / "judge.py", "GE", work / "draft-r4.md", pack_path])
ge = json.loads(r.stdout); res["ge"] = ge["pass"]
r = run([sys.executable, sb / "judge.py", "GG", work / "draft-r4.md", pack_path, "--plan", work / "d1-plan.md"])
gg = json.loads(r.stdout); res["gg"] = gg["pass"]
record(work / "ga-report-r4.json", "regauntlet",
       {"cycle": 4, **res, "ga_hard": ga["hard_failures"], "ga_labels": ga["label_failures"],
        "ge_defects": ge["defects"], "gg_defects": gg["defects"],
        "gates_run": ["G-A", "G-B", "G-C", "G-D", "G-E", "G-F", "G-G"]})
res["all"] = all(res.values())
print(json.dumps({"cycle": 4, **res,
                  "ga_hard": ga["hard_failures"], "ga_labels": ga["label_failures"],
                  "ga_advisories_n": len(ga["advisories"]),
                  "mech_defects": mech["defects"], "ge_defects": ge["defects"],
                  "gg_defects": gg["defects"]}, indent=1))
if res["all"]:
    r = run([sys.executable, sb / "provenance.py", "verify", chain])
    print(r.stdout.strip())
    r = run([sys.executable, sb / "provenance.py", "declare", chain, pack_path])
    (work / "declaration.md").write_text(r.stdout)
    json.dump({"status": "ALL GATES GREEN", "cycles": 4, "final_draft": str(work / "draft-r4.md")},
              open(work / "m6-outcome.json", "w"), indent=1)
    print("M6: ALL GATES GREEN — draft-r4.md")
