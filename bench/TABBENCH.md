# TabBench: a composition eval for AI agents

TabBench measures what code benchmarks cannot: long-horizon tool
orchestration with taste. An agent gets the same brief, the same 59-tool
MCP surface, and a live TuxGuitar score - and has to deliver a finished,
structured, playable song. Scoring combines objective instruments (the
AI Ear) with a blind human ear-vote.

Why it is a meaningful benchmark:

- Long-horizon tool use: a run takes dozens of dependent calls with
  revision checks punishing sloppy state tracking.
- Constraint satisfaction under taste: hard targets (tempo window,
  syncopation, kick unison, development quota) plus "does it slap".
- Self-critique: the AI Ear rewards agents that read the scorecard and
  act on it; the pass trend measures actual improvement.
- Not in any training set: the tools are novel, so agents must read
  schemas and reason instead of recalling answers.

## Protocol

1. Fresh empty score in TuxGuitar (bridge plugin >= 0.9.6), one MCP
   client connected at a time.
2. Give the contestant `prompt.md` verbatim (only the save-name differs).
3. The contestant runs unattended. No human hints beyond "continue".
4. After the run: save the .tg, then run `judge.py` against the open
   score to produce the objective scorecard, and keep the render for the
   blind ear-vote.
5. Blind vote: shuffle the renders under neutral names; human picks by
   ear; unseal.

## Scoring rubric (objective panel, 100 points)

| Criterion | Points | Source |
|---|---|---|
| Style match: assigned genre in top match, tempo + syncopation in window | 15 | style_match, evaluate |
| Scale adherence: detected scale matches the brief | 10 | evaluate |
| Development quota: literal-repeat share within the style's budget | 10 | evaluate DEVELOPMENT QUOTA |
| Hook: main riff passes hook_check | 15 | hook_check |
| Structure: theme map shows marked sections with at least one restated or varied motif relation | 15 | track_themes |
| Cleanliness: zero cross-track clashes; realism clean | 15 | evaluate, check_realism |
| Human feel: HUMAN-FEEL passes; velocity std >= 2 | 10 | evaluate |
| Mix: no clipping, no mud flag, no quiet holes | 10 | render_and_listen |

Human ear-vote breaks ties and is reported alongside, never blended in.

## Results so far (informal rounds, 2026-07-20)

Round 1 - basic toolkit (40 tools), 7-string A-standard metalcore brief:

| Contestant | Client | Ear-vote | Notes |
|---|---|---|---|
| Gemini | gemini-cli | 1st | winner by ear |
| Claude | Claude Code | 2nd | |
| Codex | Codex CLI | 3rd | found the palmMute/staccato bug |

Round 2 - upgraded toolkit (52 tools incl. generate_riff, hook_check,
interlock, themes), same brief:

| Contestant | Client | Panel | Ear-vote | Notes |
|---|---|---|---|---|
| Claude | Claude Code | 1st | shortlisted | only entry matching the brief: 85% metalcore, hook PASS, 0 clashes, 5 tracks |
| Codex | Codex CLI | 2nd | - | best phrasing (only call-and-response), but genre drift to classic heavy + copy-paste |
| Gemini | Antigravity | 3rd | shortlisted | 15 tritone clashes, hook REJECTED, no markers |

Observations that shaped the toolkit: copy-paste dominance led to the
development quota; empty-track noise led to the evaluate skip; "revision
jumped" reports led to coalescing.

## Files

- `prompt.md` - the canonical brief (edit the save-name per contestant)
- `judge.py` - runs the objective panel against the open score and
  prints the rubric scores
- Results land in `results/<date>-<contestant>.txt`
