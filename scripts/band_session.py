#!/usr/bin/env python3
"""Band simulation v2: five INDEPENDENT model instances review the score
open in TuxGuitar, each from its own role with its own tools and its own
context, then a producer's chair applies the changes with broadest
support. Unlike the `band` MCP prompt (one model playing five parts),
every role here is a separate headless Claude Code run - different
contexts, optionally different models.

Usage:
  python3 scripts/band_session.py
  python3 scripts/band_session.py --roles critic,listener --no-apply
  TABMCP_BAND_MODELS="critic=opus,listener=haiku" python3 scripts/band_session.py

Roles run sequentially (the bridge serves one client at a time).
Requires: claude CLI, tabmcp installed, TuxGuitar running with a score.
"""
import json
import os
import shutil
import subprocess
import sys

CLAUDE = None
for candidate in [
    os.path.expanduser("~/.local/bin/claude"),
    os.path.expanduser("~/.claude/local/claude"),
    "/opt/homebrew/bin/claude",
    "/usr/local/bin/claude",
    shutil.which("claude") or "",
]:
    if candidate and os.access(candidate, os.X_OK):
        CLAUDE = candidate
        break
if CLAUDE is None:
    print("claude binary not found")
    sys.exit(1)

TABMCP = os.path.expanduser("~/.cargo/bin/tabmcp")
MCP_CONFIG = json.dumps(
    {"mcpServers": {"tuxguitar": {"command": TABMCP, "args": ["serve"]}}})

VOTE_FORMAT = (
    "End your reply with EXACTLY one line of JSON on its own line: "
    '{"vote": "<the ONE concrete change you want most, as an imperative '
    'sentence with measure/track numbers>", "reason": "<one sentence>"}'
)

ROLES = [
    ("composer",
     "You are the COMPOSER of this band. Review the score open in TuxGuitar "
     "using ONLY tuxguitar_track_themes, tuxguitar_riff_dna and "
     "tuxguitar_get_score_summary. Judge motif development and musical "
     "memory: does the song develop its material or forget it? "),
    ("critic",
     "You are the CRITIC - never polite, only honest. Review using ONLY "
     "tuxguitar_evaluate (style='metalcore' if it fits, else no style) and "
     "tuxguitar_hook_check on the main riff. Would anyone remember this "
     "tomorrow? "),
    ("producer",
     "You are the PRODUCER. Review using ONLY tuxguitar_producer_notes, "
     "tuxguitar_style_match and tuxguitar_get_score_summary. Judge the "
     "arrangement arc: builds, drops, transitions, section weights. "),
    ("guitarist",
     "You are the GUITARIST who has to play this. Review using ONLY "
     "tuxguitar_check_realism and tuxguitar_analyze_difficulty. Can hands "
     "play this for a full take? "),
    ("listener",
     "You are the LISTENER - you only care how it sounds. Review using ONLY "
     "tuxguitar_render_and_listen and tuxguitar_listen_stems. Any "
     "instrument buried, boring or muddy? "),
]

only_roles = None
apply_votes = True
for i, arg in enumerate(sys.argv):
    if arg == "--roles" and i + 1 < len(sys.argv):
        only_roles = {r.strip() for r in sys.argv[i + 1].split(",")}
    if arg == "--no-apply":
        apply_votes = False

model_overrides = {}
for pair in os.environ.get("TABMCP_BAND_MODELS", "").split(","):
    if "=" in pair:
        role, model = pair.split("=", 1)
        model_overrides[role.strip()] = model.strip()


def run_turn(role, prompt, extra_args=None):
    command = [
        CLAUDE, "-p", prompt,
        "--output-format", "json",
        "--mcp-config", MCP_CONFIG,
        "--strict-mcp-config",
        "--allowedTools", "mcp__tuxguitar",
    ]
    if role in model_overrides:
        command += ["--model", model_overrides[role]]
    if extra_args:
        command += extra_args
    result = subprocess.run(command, capture_output=True, text=True, timeout=1800)
    if result.returncode != 0:
        return None, f"exit {result.returncode}: {result.stderr[-300:]}"
    try:
        payload = json.loads(result.stdout)
        return payload.get("result", ""), None
    except Exception:
        return result.stdout, None


minutes = ["BAND MEETING MINUTES", "===================="]
votes = []
for role, persona in ROLES:
    if only_roles and role not in only_roles:
        continue
    print(f"[{role}] reviewing...", flush=True)
    text, error = run_turn(role, persona + VOTE_FORMAT)
    if error or text is None:
        minutes.append(f"\n{role.upper()}: FAILED - {error}")
        continue
    vote = None
    for line in reversed(text.strip().splitlines()):
        line = line.strip().strip("`")
        if line.startswith("{") and '"vote"' in line:
            try:
                vote = json.loads(line)
                break
            except Exception:
                continue
    minutes.append(f"\n{role.upper()} says:\n{text.strip()[:1200]}")
    if vote:
        votes.append((role, vote.get("vote", ""), vote.get("reason", "")))
        print(f"[{role}] vote: {vote.get('vote','')[:100]}", flush=True)
    else:
        print(f"[{role}] no parseable vote", flush=True)

if votes and apply_votes:
    ballot = "\n".join(
        f"- {role}: {vote} (because: {reason})" for role, vote, reason in votes)
    chair_prompt = (
        "You are the PRODUCER'S CHAIR of a band session. The band reviewed "
        "the score open in TuxGuitar and voted for changes:\n" + ballot +
        "\n\nPick the ONE or TWO changes with the broadest support or biggest "
        "musical payoff, apply them with the tuxguitar edit tools (preview "
        "then confirm - every edit must actually be applied, not just "
        "described), then run tuxguitar_evaluate once to confirm nothing got "
        "worse. Report what you changed and why in plain sentences.")
    print("[chair] applying the vote...", flush=True)
    text, error = run_turn("chair", chair_prompt)
    minutes.append("\nPRODUCER'S CHAIR:\n" + (text.strip()[:2000] if text else f"FAILED - {error}"))
elif votes:
    minutes.append("\n(--no-apply: votes collected, nothing applied)")
else:
    minutes.append("\nNo votes parsed - nothing applied.")

output = "\n".join(minutes)
print("\n" + output[:3000])
out_path = os.path.expanduser("~/.tuxguitar-mcp/band-minutes.txt")
os.makedirs(os.path.dirname(out_path), exist_ok=True)
with open(out_path, "w") as f:
    f.write(output)
print(f"\nfull minutes: {out_path}")
