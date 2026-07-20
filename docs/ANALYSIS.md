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

1. **Riff-device transforms as tools** — `vary_riff { transform }`:
   rhythmic displacement (shift pattern by an 8th/16th), octave
   displacement, inversion, retrograde, diminution/augmentation (halve or
   double note values), pedal-tone interleave. Mechanical on the wire
   model; instantly multiplies material from any 1-bar seed.
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

## 3. The style system — `tuxguitar_style_guide` (recommended next build)

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

## 4. Judgment upgrades (evaluators that reward complexity)

- **Syncopation score**: fraction of onsets off the strong beats — flag
  BOTH extremes ("metronomic" vs "unmoored"), styled thresholds.
- **Motif development**: today repetition is one number; split into
  *literal repeats* vs *varied repeats* (same rhythm, different pitches or
  vice versa) — reward variation, not copy-paste.
- **Tension curve**: per-measure dissonance + register + density composite
  plotted across sections; "your breakdown releases tension before the
  climax" style notes.
- **Harmonic rhythm**: chord-change rate per section.
- **Pass history**: evaluate stores per-pass scores in the session so the
  loop reports trends ("groove 53% → 84%").

## 5. Everything else, prioritized

**Near (mechanical):** revision-bump coalescing (Java events); tremolo-bar
& grace params; vary_riff transforms; style_guide tool; per-section
evaluate for ALL metrics (currently groove only).

**Middle (design):** chord-progression planner (name a progression, get
voicings per style + voice-leading check); solo scaffolding (contour plan +
lick cells + the fingering optimizer); A/B render compare
(render.wav vs render-prev.wav with a diff report); named checkpoints
(save-copy automation per pass).

**Far (research):** audio-to-tab Stage 2 (Demucs + basic-pitch);
headless .tg engine; MCP resources/elicitation; TuxGuitar 2.1 migration.

**Explicitly parked by user:** loop-practice transport, distribution.
