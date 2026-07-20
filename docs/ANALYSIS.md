# TabMCP — Depth Analysis: Complexity, Styles, and What's Next

*2026-07-20, after three field sessions (E-minor demo, Phrygian/Hirajoshi
piece, 20-bar structured song with 7/8).*

## 1. Where riff complexity actually comes from

Three layers produce complexity, and they improve differently:

| Layer | Lives in | Current state | How to improve |
|---|---|---|---|
| **Vocabulary** — what CAN be written | Server (wire model, tools) | Strong: chords, tuplets, 2 voices, odd meters, pinch harmonics, bend curves, 44 scales | Close the last gaps (below) |
| **Knowledge** — what the AI knows to TRY | AI client + server hints | Client's own musicianship + tool descriptions | **Style guide system** (§3) — the biggest lever |
| **Judgment** — what gets REWARDED | Server evaluators | Groove, motifs, clashes, dynamics, sections | Complexity-aware metrics (§4) |

Field evidence: sessions already write reverse gallops, 2+2+3 groupings,
mode pivots and meter changes **when the prompt names them**. The model is
not short of musicianship — it is short of *prompted vocabulary*. A style
guide the AI can query turns "write something complex" into "use these
seven named devices", which is how human players learn too.

## 2. Vocabulary gaps (server work, ordered by payoff)

1. ~~**Riff-device transforms as tools**~~ ✅ DONE: `vary_riff` now has 9
   transforms — displace, retrograde, invert, octave, augment, diminish,
   pedal-tone fill (palm-muted), polymetric regroup (3+3+2 accents across
   barlines), dynamics swap (mutes <-> accents).
2. **Remaining effect parameters on write**: grace note (fret,
   duration, transition), trill (fret, speed), tremolo-picking speed,
   tremolo-bar curves (same point-list shape as bends). Read side flags
   exist; write side ignores them.
3. **Second-voice authoring guidance**: voice 1 melody over voice 0 chug
   already round-trips; no tool composes into voice 1 deliberately.
4. **Polymeter support**: different track meters aren't possible in
   TuxGuitar (headers are song-wide), but *implied* polymeter (3-note
   pattern over 4/4) is a vary_riff transform.
5. **Groove feel**: TuxGuitar's grid has no micro-timing, so swing must be
   written as tuplet pairs (triplet swing) — a `feel` option on humanize
   / drum templates (velocity-based push-pull is already possible).

## 3. The style system — `tuxguitar_style_guide` (✅ SHIPPED, then extended)

Now 16 styles (deathcore added) on a consistent rubric: scales, tuning
presets, tempo + numeric range, meters, rhythmic cells, techniques, drums,
signature devices, song-section arc, mood, difficulty, an AVOID list, and
numeric evaluation targets (tempo + syncopation window) that
`tuxguitar_evaluate { style }` checks automatically. Original design table
below for reference:

One read-only tool: `style_guide { style }` returns a composition recipe
the AI folds into its writing. Data table, ~15 lines per style. Proposed
catalog v1:

| Style | Scales | Tempo | Rhythmic cells | Techniques | Drums | Devices |
|---|---|---|---|---|---|---|
| **thrash** | natural/harmonic minor, chromatic passing | 160–220 | straight 16th chug, gallop | palm mute, fast alt picking | d-beat, blast fills | E5-F5 semitone stabs, tritone riffs |
| **death metal** | phrygian, locrian, diminished | 180–260 | blast 16ths, tremolo | tremolo picking, pinch | blast | chromatic descent, tritone pedal |
| **black metal** | natural minor, phrygian | 180–240 | straight 8th/16th tremolo | tremolo picked chords | blast (rawer) | minor-chord arpeggios tremolo'd |
| **doom / sludge** | minor pentatonic, phrygian | 55–90 | half/whole notes, triplet drags | bends, slides, let ring | halftime | tritone (Iommi) bends, space |
| **groove metal** | phrygian dominant, blues | 90–130 | 16th syncopation, 2+2+3 | palm mute, pinch squeals | halftime, rock | rhythmic displacement of one riff |
| **djent / prog** | phrygian dominant, altered, chromatic | 110–150 | 7/8, 5/4, 2+2+3+2, polymetric 3s over 4/4 | low-string chug, wide-interval melody on top | gallop + halftime switches | same riff re-barred across meters |
| **metalcore** | minor, drop-tuning power chords | 120–160 | breakdown half-time 8ths | open chug, dissonant octave stabs | halftime breakdowns | the "china + chug" unison |
| **power metal** | major, harmonic minor | 160–200 | straight gallop | fast scale runs, harmonized 3rds | gallop | dual-lead harmony (generate_harmony!) |
| **classic heavy / NWOBHM** | minor pentatonic, dorian | 120–160 | 8th-note drive | double stops, unison bends | rock | harmonized leads, gallop bridges |
| **punk / hardcore** | major/minor pentatonic | 160–210 | straight 8ths | downpicked power chords | punk, d-beat | 3-chord turnarounds |
| **blues rock** | blues, mixolydian | 80–140 | shuffle (triplet pairs), 12-bar | bends, vibrato, slides | rock (triplet feel) | call-and-response, turnaround licks |
| **funk rock** | dorian, minor pentatonic | 95–115 | 16th ghost-note grid | ghost notes, staccato, dead notes | rock (ghosted) | single-note riffs w/ chromatic approaches |
| **jazz fusion** | melodic minor modes, altered, lydian dominant | 100–180 | swung 8ths, odd groupings | legato, wide intervals | ride-driven | ii-V colors, superimposition |
| **flamenco metal** | phrygian dominant, double harmonic | 100–140 | triplet rasgueado feel | fast triplet picking | halftime/rock | Andalusian cadence (Am-G-F-E) |
| **surf / spy** | hirajoshi, harmonic minor | 140–180 | straight 8th tremolo | tremolo picking, spring verb implied | rock | minor-key double picking |

Every column maps to EXISTING tools: scales → the 44-scale catalog, drums
→ the template styles, techniques → effect objects, cells → replace_measures
patterns. The guide is glue, not new machinery. A `list` variant returns
the catalog names.

## 4. Judgment upgrades (evaluators that reward complexity) — ✅ ALL DONE

- ~~**Syncopation score**~~ ✅ weighted off-beat share, style-windowed via
  `evaluate { style }` (each of the 16 rubrics carries a target range).
- ~~**Motif development**~~ ✅ literal / varied / fresh measure counts;
  copy-paste flagged as an ISSUE, variation rewarded.
- ~~**Tension curve**~~ ✅ per-measure composite (density + dynamics +
  register + dissonance) with a sparkline, flat-curve ISSUE, and optional
  `tension_target` arc comparison.
- ~~**Harmonic rhythm**~~ ✅ root-change rate per measure boundary.
- ~~**Pass history**~~ ✅ PASS TREND line ("groove 53% -> 84%, issues 5 -> 0").
- **NEW — human-feel check** ✅: composite AI-artifact detection (identical
  velocities, copy-paste-only repeats, flat energy, no motif).

## 5. Everything else, prioritized

**Near (mechanical):** revision-bump coalescing (Java events); tremolo-bar
& grace params; per-section evaluate for ALL metrics (currently groove
only); `evolve_riff` generation driver (#6) and `analyze_difficulty` (#8)
from the idea backlog below. ~~vary_riff transforms; style_guide tool~~ ✅ done.

**Middle (design):** chord-progression planner (name a progression, get
voicings per style + voice-leading check); solo scaffolding (contour plan +
lick cells + the fingering optimizer); A/B render compare
(render.wav vs render-prev.wav with a diff report); named checkpoints
(save-copy automation per pass).

**Far (research):** audio-to-tab Stage 2 (Demucs + basic-pitch);
headless .tg engine; MCP resources/elicitation; TuxGuitar 2.1 migration.

**Explicitly parked by user:** loop-practice transport, distribution.

## 6. The idea backlog (2026-07-20 session, user's 12)

Status after the "beast batch" (vary_riff x9, evaluator upgrades, style
rubrics, riff_dna):

| # | Idea | Status |
|---|---|---|
| 1 | **Riff DNA** — extract motif/rhythm/scale/techniques/energy identity | ✅ `tuxguitar_riff_dna` shipped |
| 2 | **Section awareness** — what each song section should do | ✅ `sections` field in every style rubric; per-section groove already in evaluate |
| 3 | **Instrument roles** — lead/rhythm/bass/kick/snare purposes | Partial: generators encode roles (bass mirrors accents, drums templates); a `roles` guide field is a cheap next step |
| 4 | **Tension curve targets** | ✅ measured curve + `tension_target` compare in evaluate |
| 5 | **Humanization score** | ✅ robotic-velocity, literal-repeat, flat-tension flags feed the human-feel check |
| 6 | **Riff evolution** — generations with mutations | Next: the loop exists manually (riff_dna -> vary_riff -> evaluate); a `evolve_riff` driver tool would automate N generations |
| 7 | **Technique budget** — percentage mix per style | riff_dna reports the measured mix; enforcing a target mix = evaluator addition |
| 8 | **Difficulty analyzer** — 1-10 with reasons | Next: fingering CostModel already computes effort; expose as `analyze_difficulty` |
| 9 | **Explain every decision** | Partial: fingering explains itself; edit tools return summaries; a "reason" convention for generators would complete it |
| 10 | **"Could a human have written this?"** | ✅ HUMAN-FEEL CHECK in evaluate (AI-artifact detection) |
| 11 | **Riff genealogy** — family tree of variants | Design: needs named checkpoints/branches; pairs with #6 |
| 12 | **Band simulation** — role agents + producer + AI Ear | Long-term: the MCP surface already supports it (any client can run multi-agent); a `band` MCP prompt could orchestrate |
