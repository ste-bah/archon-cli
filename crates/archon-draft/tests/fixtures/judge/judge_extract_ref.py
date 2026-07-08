#!/usr/bin/env python3
"""Verbatim mirror of judge.py's battery + seed + shuffle + extraction (no API).
usage: judge_extract_ref.py <GE|GG> <section_id> <text-file>  -> report JSON"""
import hashlib, json, re, sys
gate, section_id, text = sys.argv[1], sys.argv[2], open(sys.argv[3]).read()

if gate == "GE":
    battery = [
        ("E1", "Is every claim of the foundation text present in the passage (none silently dropped)?", "NO"),
        ("E2", "Does the passage alter any foundation claim's strength or modality beyond what its evidence grade licenses (e.g., asserting an UNCERTAIN item flatly)?", "YES"),
        ("E3", "Does the passage assert anything with no warrant in the evidence bank or foundation text?", "YES"),
        ("E4", "Is each quotation used in a way consistent with its stated intended rhetorical use in the quote index?", "NO"),
        ("E5", "Are all evidence items asserted at a strength matching their grade (AUTHOR-CONFIRMED flat; CONFIRMED with its measure; UNCERTAIN hedged or omitted)?", "NO"),
    ]
else:
    battery = [
        ("G1", "Is every locked term used per its locked definition at every occurrence?", "NO"),
        ("G2", "Does any later passage contradict an earlier passage's claim without explicit acknowledgment?", "YES"),
        ("G3", "Is each evidence item characterized with the same value/strength everywhere it appears?", "NO"),
        ("G4", "Do any two passages characterize the same concept incompatibly?", "YES"),
    ]

seed = int(hashlib.sha256(section_id.encode()).hexdigest()[:8], 16)
order = list(range(len(battery)))
r = seed
for i in range(len(order) - 1, 0, -1):
    r = (r * 1103515245 + 12345) % (2**31)
    j = r % (i + 1)
    order[i], order[j] = order[j], order[i]
battery = [battery[i] for i in order]

defects, transcript = [], []
starts = []
for i in range(len(battery)):
    m = re.search(rf"^\s*(?:\*\*)?{i+1}[\.\)]", text, flags=re.M)
    starts.append(m.start() if m else None)
for i, (code, q, bad_on) in enumerate(battery):
    if starts[i] is None:
        block = ""
    else:
        nxt = next((s for s in starts[i + 1:] if s is not None), len(text))
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
print(json.dumps({"battery_order": [b[0] for b in battery], "defects": defects, "transcript": transcript}, indent=1))
