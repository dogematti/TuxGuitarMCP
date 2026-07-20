# TabMCP - Depth Analysis: Complexity, Styles, and What's Next

Updated 2026-07-20 after three field sessions (E-minor demo,
Phrygian/Hirajoshi piece, 20-bar structured song with 7/8), TWO
cross-model riff battles (round 1: Gemini won by ear; round 2 on the
upgraded toolkit: Claude won by ear + judge panel, Codex best phrasing
both times), the composition-intelligence batches, the complexity
engines, player notes, and the embedded chat.

## 1. Where riff complexity actually comes from

Three layers produce complexity, and they improve differently:

| Layer | Lives in | Current state |
|---|---|---|
| Vocabulary - what CAN be written | Server (wire model, tools) | Strong: chords, tuplets, 2 voices, odd meters, pinch harmonics, bend curves, 44 scales, 9 riff transforms |
| Knowledge - what the AI knows to TRY | AI client + server hints | 16-style rubric system, blends, instrument roles, riff DNA |
| Judgment - what gets REWARDED | Server evaluators | Full AI Ear v2: groove, syncopation, motif development, tension, harmonic rhythm, rests, boredom, surprise, human-feel, hook gate |

Field evidence: sessions write reverse gallops, 2+2+3 groupings, mode
pivots and meter changes when the prompt names them. The model is not
short of musicianship - it is short of prompted vocabulary. The style
guide turns "write something complex" into "use these named devices",
which is how human players learn too.

## 2. Shipped systems (reference)

### Transforms (tuxguitar_vary_riff)

Nine mechanical mutations, meter-aware, preview/confirm, undoable:
displace (any tick amount - odd values give polyrhythms), retrograde,
invert (chromatic mirror around the first note), octave (with fretboard
refitting), augment (durations double, range doubles), diminish
(durations halve, material compresses), pedal (palm-muted pedal-tone fill
on the free grid), regroup (3+3+2-style accents cycling across barlines -
implied polymeter), swap_dynamics (palm-mutes and accents trade places).

### The style system (tuxguitar_style_guide)

16 styles (thrash, death metal, black metal, doom, groove metal, djent,
metalcore, deathcore, power metal, classic heavy, punk, blues rock, funk
rock, jazz fusion, flamenco metal, surf) on one consistent rubric:
scales, tuning presets, tempo plus numeric range, meters, rhythmic cells,
techniques, drum styles, signature devices, song-section arc, mood,
difficulty, an AVOID list, and numeric evaluation targets (tempo +
syncopation window) that `tuxguitar_evaluate { style }` checks
automatically. Genre crossover via blend syntax ("60% death metal, 40%
doom") merges characteristics by weight - traits, never copied riffs.
Universal instrument roles ship with the catalog listing.

### The AI Ear (tuxguitar_evaluate and friends)

Per track: groove periodicity, syncopation (weighted off-beat share),
motif development (literal / varied / fresh measures), tension curve
(density + dynamics + register + dissonance per measure, with sparkline),
harmonic rhythm (root-change rate), rest share, velocity statistics,
surprise events (pattern breaks after established repetition), contour
statistics (stepwise share, pitch span). Cross-track: clashes, masking,
alignment, empty bars, key/scale. Composites: boredom risk (five signals),
human-feel check (AI-artifact detection), pass-over-pass trend. Optional
targets: per-style windows, tension arcs ("0.2,0.5,1.0"), emotion
journeys ("calm, uneasy, aggressive, victorious").

### Composition intelligence tools

- riff_dna: motif, rhythm cell, scale, register, technique mix, energy
  1-10, harmonic motion - the identity card for evolving without copying.
- evolve_riff: deterministic hill-climb through mutation generations,
  AI Ear fitness, lineage report, two-step write.
- track_themes: musical memory - which section introduces motif A, which
  restate/vary/invert/retrograde/fragment/extend it, call-and-response
  pairs, and a flag when the song forgets its own material.
- hook_check: the memorability gate - six criteria (motif, hummable
  contour, rhythmic identity, dynamics, breathing room, surprise), four
  to pass, rejections listed. "Would anyone remember this tomorrow?"
- check_realism: impossible stretches, duplicate strings per beat, frets
  past the neck, string-breaking bends, stiff wound-string bends,
  open-string/high-position mixes.
- analyze_difficulty: 1-10 with itemized reasons - notes/sec, chord
  stretches, fast shifts, string skips, 16th-run endurance, finger
  fatigue (consecutive measures with no breather), picking simulation
  (sweep shapes, zigzag crossings), technique load.
- producer_notes: arrangement-level suggestions (double the riff, strip
  before the climax, harmonize the peak, reduced sections, breakdown
  unison checks), each naming the tool that executes it.
- style_match: measured characteristics ranked against every rubric -
  "what makes this sound like death metal" without touching band riffs.

### Complexity engines (the batch after the intelligence tools)

- Rhythm-cell catalog: 16 named cells (gallop, reverse-gallop, herta,
  tresillo, hemiola, triplet-8ths, quintuplet, dotted-8ths, rests, ...)
  with exact tick patterns, dots and tuplets - rhythm as a compositional
  unit. Spelled onto measures without straddling barlines.
- generate_riff: constraint-guided beam search. Phase 1 fills each bar
  with cells (density window, syncopation window, accent coverage);
  phase 2 assigns pitches from the scale-in-register pool (roots on
  accents, stepwise motion with b2/tritone spice, bounded repeat runs,
  motif echo of bar one). AABA' form; velocities and palm mutes shaped;
  fingered by the optimizer. Deterministic.
- rebar: pour a passage across a different meter structure - barlines
  move, notes keep their flow (set_time_signature first, then rebar).
- generate_counterline: answering melody in the source's gaps, contrary
  motion, consonant on strong beats, an octave up.
- generate_interlock: drums derived from the riff (kick in unison with
  its accents, backbeat snare in any meter, 8th hats).
- DNA bank: riff_dna save_as writes identity cards to
  ~/.tuxguitar-mcp/dna_bank.jsonl; list_bank recalls them in any session.
- Write-side articulations (plugin 0.8.0): tremolo picking, trills and
  grace notes with parameters (see PROTOCOL.md).

### Knowledge and embedding (the batches after the complexity engines)

- Player notes: ~/.tuxguitar-mcp/styles/<style>.md and tuning-specific
  <style>.<tuning-prefix>.md files served by style_guide (tuning
  auto-detected from the open score); precedence tuning notes > base
  notes > built-in rubric; a new file name defines a custom style.
  The user's fretboard vocabulary teaches every AI client at once.
- Embedded chat (plugin 0.9.0): Tools -> "TabMCP: AI Musician Chat"
  opens a chat window inside TuxGuitar backed by headless Claude Code
  turns (stream-json, strict inline MCP config, tuxguitar tools
  auto-allowed, session continuity via --continue). Field-verified:
  full 8-bar arrangement composed, self-corrected, and played back in
  one turn from inside the TuxGuitar window.

### Prompts

compose (style/key/bars), refine (N AI Ear passes), band (five
personalities - composer, critic, producer, guitarist, listener - review
with their own tools, vote, apply the winners).

## 3. Idea backlog - first batch (user's 12)

| # | Idea | Status |
|---|---|---|
| 1 | Riff DNA | DONE - tuxguitar_riff_dna |
| 2 | Section awareness | DONE - sections field in every rubric; per-section groove in evaluate |
| 3 | Instrument roles | DONE - ROLES text with the style catalog |
| 4 | Tension curve targets | DONE - tension_target in evaluate |
| 5 | Humanization score | DONE - robotic-velocity, literal-repeat, flat-tension signals |
| 6 | Riff evolution | DONE - tuxguitar_evolve_riff (mechanical); pitch-level evolution stays client-side via riff_dna |
| 7 | Technique budget | PARTIAL - riff_dna reports the measured mix; enforcing a target mix is an evaluator addition |
| 8 | Difficulty analyzer | DONE - tuxguitar_analyze_difficulty |
| 9 | Explain every decision | PARTIAL - fingering explains itself, edits return summaries; a reason convention for generators remains |
| 10 | Could a human have written this | DONE - HUMAN-FEEL CHECK in evaluate |
| 11 | Riff genealogy | OPEN - needs named checkpoints/branches; pairs with 6 |
| 12 | Band simulation | DONE (v1) - the band prompt orchestrates five personalities; true multi-agent stays client-side |

## 4. Idea backlog - second batch (user's 16)

| Idea | Status |
|---|---|
| Musical memory | DONE - tuxguitar_track_themes |
| Call and response detector | DONE - inside track_themes |
| "What makes this sound like X" | DONE - tuxguitar_style_match (characteristics, never riffs) |
| Boringness detector | DONE - BOREDOM RISK composite in evaluate |
| Rest analyzer | DONE - rest_share metric + breathing-room issue |
| Finger fatigue model | DONE - fatigue streak in analyze_difficulty |
| Picking simulator | DONE (v1) - sweep shapes, zigzag crossings, downpick endurance in analyze_difficulty |
| Genre crossover | DONE - blend syntax in style_guide |
| Surprise meter | DONE - surprise_breaks + predictable/chaotic flags in evaluate |
| Theme tracker | DONE - track_themes relations (restate, vary, invert, retrograde, fragment, extend) |
| Guitar realism checker | DONE - tuxguitar_check_realism |
| Producer mode | DONE - tuxguitar_producer_notes |
| Multiple personalities | DONE (v1) - band prompt |
| Emotion target | DONE - emotion_target vocabulary in evaluate |
| Live audience simulation | SKIPPED by user |
| Classic-riff gate | DONE - tuxguitar_hook_check |

## 5. Roadmap v3: the arrangement layer (analysis of 2026-07-20 evening)

Phase one taught the system to write a riff; phase two taught it to judge
one. The remaining complexity lives BETWEEN riffs. Build order:

1. Transition engine: drum fills, buildups, and band stops at section
   boundaries (markers) - the cheapest big audible win.
2. Ornament pass: style-idiomatic articulation over existing material
   (vibrato, slides, pinch on peaks, tremolo conversion; realism-gated).
   Generators currently use ZERO of the articulations the plugin can
   write since 0.8.0.
3. plan_harmony: progression planning + voice-leading check; battle
   evidence shows every entry pedals one root for the whole song. Plus
   development quotas in style targets (max literal-repeat share) so
   evaluate { style } fails loudly on copy-paste.
4. Lick-cell lead generator: pitch-shape cells + contour plan +
   question/answer periods, fingering-optimized, difficulty-capped.
5. Tension-coupled search: generate_riff fitness follows a per-bar
   tension/emotion target; phrase-end cadence awareness.
6. Plumbing: persist AI Ear pass history to disk keyed by documentId
   (in-memory state dies every embedded-chat turn - confirmed);
   diff_measures (musical git-diff); per-section evaluate for ALL
   metrics; structured player-note hints parsed into generate_riff
   constraints.

Deliberately not next: more styles (depth over breadth), micro-timing
(grid cannot hold it), audio-to-tab stage 2 (research-sized).

## 6. What remains, prioritized (older list)

Near (mechanical): revision-bump coalescing (Java events); tremolo-bar
curves on write (grace/trill/tremolo-picking shipped in 0.8.0);
per-section evaluate for ALL metrics (currently groove only);
technique-budget enforcement (idea 7); generator reason lines (idea 9);
DNA-bank-driven generation bias (feed saved cards into generate_riff
constraints).

Middle (design): riff genealogy (named checkpoints and branch trees, idea
11); chord-progression planner (name a progression, get voicings per
style plus a voice-leading check); solo scaffolding (contour plan + lick
cells + the fingering optimizer); A/B render compare with a diff report.

Far (research): audio-to-tab Stage 2 (Demucs + basic-pitch); headless
.tg engine; MCP resources/elicitation; TuxGuitar 2.1 migration; true
multi-agent band simulation with independent model instances per role.

Explicitly parked by user: loop-practice transport, distribution
packaging, live audience simulation.
