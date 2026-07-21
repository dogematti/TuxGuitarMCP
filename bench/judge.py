#!/usr/bin/env python3
"""TabBench objective judge: run the AI Ear panel against the score open
in TuxGuitar and print rubric scores (see TABBENCH.md).

Usage: python3 bench/judge.py <contestant-name> [--out results/]
"""
import json
import os
import re
import subprocess
import sys

CONTESTANT = sys.argv[1] if len(sys.argv) > 1 else "entry"
OUT_DIR = sys.argv[3] if len(sys.argv) > 3 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "results")
BIN = os.path.expanduser("~/.cargo/bin/tabmcp")

proc = subprocess.Popen([BIN, "serve"], stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                        text=True)
mid = [0]


def send(method, params=None, notify=False):
    m = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        m["params"] = params
    if not notify:
        mid[0] += 1
        m["id"] = mid[0]
    proc.stdin.write(json.dumps(m) + "\n")
    proc.stdin.flush()
    if notify:
        return None
    while True:
        line = proc.stdout.readline()
        if not line:
            print("EOF from server")
            sys.exit(1)
        d = json.loads(line)
        if d.get("id") == mid[0]:
            return d


send("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                    "clientInfo": {"name": "tabbench", "version": "2"}})
send("notifications/initialized", notify=True)

transcript = []


def call(name, args):
    r = send("tools/call", {"name": name, "arguments": args})
    if "error" in r:
        transcript.append(f"--- {name} ---\nERROR: {r['error']['message'][:300]}")
        return ""
    text = "\n".join(c.get("text", "") for c in r["result"].get("content", []))
    transcript.append(f"--- {name} ---\n{text}")
    return "" if r["result"].get("isError") else text


style_match = call("tuxguitar_style_match", {})
evaluate = call("tuxguitar_evaluate", {"style": "metalcore"})
themes = call("tuxguitar_track_themes", {})
realism = call("tuxguitar_check_realism", {})
hook = call("tuxguitar_hook_check",
            {"track_number": 1, "from_measure": 5, "to_measure": 8})
render = call("tuxguitar_render_and_listen", {})

scores = {}

# Style match (15): metalcore top-ranked + tempo/syncopation in window.
top_line = next((l for l in style_match.splitlines() if "%" in l), "")
scores["style"] = (
    (8 if "metalcore" in top_line else 0)
    + (4 if "tempo" in evaluate and "OK, within" in evaluate else 0)
    + (3 if "OK, in the style's window" in evaluate else 0)
)

# Scale adherence (10): natural minor detected.
scores["scale"] = 10 if re.search(r"Key/scale: .*(natural minor)", evaluate) else 0

# Development quota (10).
scores["development"] = 0 if "DEVELOPMENT QUOTA FAILED" in evaluate else 10

# Hook (15).
scores["hook"] = 15 if "PASS" in hook and "REJECTED" not in hook else 0

# Structure (15): named sections + at least one motif relation.
relations = len(re.findall(r"(restates|varies|inverts|retrogrades|fragments|extends) motif",
                           themes))
named_sections = len(re.findall(r"  \w[^(]*\(m\d+-\d+\):", themes))
scores["structure"] = min(15, (8 if relations >= 1 else 0)
                          + (7 if named_sections >= 3 else 0))

# Cleanliness (15): no clashes, realism clean.
no_clash = "No harsh cross-track dissonances" in evaluate
clean_realism = "impossible" not in realism or " 0 impossible" in realism
scores["clean"] = (10 if no_clash else 0) + (5 if clean_realism else 0)

# Human feel (10).
scores["feel"] = 10 if "HUMAN-FEEL CHECK: passes" in evaluate else 0

# Mix (10): no clipping/mud/quiet holes.
scores["mix"] = (
    (4 if "clipped" not in render.lower() or "0 clipped" in render else 4)
    - (0 if "LOW-END MUD" not in render else 4)
    + (3 if "No unexpected quiet holes" in render else 0)
    + 3
)
scores["mix"] = max(0, min(10, scores["mix"]))

total = sum(scores.values())
lines = [f"TABBENCH SCORE: {CONTESTANT} - {total}/100", ""]
for key, label, maximum in [
    ("style", "Style match", 15), ("scale", "Scale adherence", 10),
    ("development", "Development quota", 10), ("hook", "Hook check", 15),
    ("structure", "Structure/themes", 15), ("clean", "Cleanliness", 15),
    ("feel", "Human feel", 10), ("mix", "Mix", 10),
]:
    lines.append(f"  {label:<18} {scores[key]:>3}/{maximum}")
summary = "\n".join(lines)
print(summary)

os.makedirs(OUT_DIR, exist_ok=True)
import datetime
stamp = datetime.date.today().isoformat()
path = os.path.join(OUT_DIR, f"{stamp}-{CONTESTANT}.txt")
with open(path, "w") as f:
    f.write(summary + "\n\n" + "\n\n".join(transcript))
print(f"\nfull transcript: {path}")
proc.stdin.close()
proc.wait(timeout=10)
