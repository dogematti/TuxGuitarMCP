# TabMCP Roadmap

Phases 0-6 are complete (see PLAN.md and git history): bridge, read/analysis,
write path, transpose, tracks/tuning/playback, fingering optimizer.

## Phase 7 — Parameterized effects (in progress)

The wire model's presence-flags become real parameter objects, so AI clients
can write expressive tablature, not just note grids.

- **Harmonics** (this phase's headline): `effects.harmonic: { "type":
  "natural" | "artificial" | "tapped" | "pinch" | "semi", "data": <octave
  offset, artificial/tapped only> }`. Pinch harmonics ("P.H") included —
  read, write, round-trip.
- **Bends**: `effects.bend: { "points": [{ "position": 0-12, "value":
  semitones }] }` — position spans the note's duration, value is the bend
  height. An empty points list applies a standard full-tone bend.
- Backward compatible: the old boolean form is still accepted on read
  (bool true maps to a natural harmonic / standard bend).
- Golden round-trip test: write pinch harmonic + bend via the bridge, read
  back, byte-compare the effect objects.
- Deferred to 7.x: tremolo-bar curves, grace-note parameters, trill speed.

## Phase 8 — Generation

- `generate_bassline`: root-following bass from detected harmony, rhythm
  locked to the guitar's accent pattern.
- `generate_harmony_track`: diatonic 3rds/6ths harmony of a monophonic lead,
  written to a new track (uses the fingering optimizer for playability).
- `generate_drum_part`: kick/snare mapped to the guitar accents (percussion
  channel), basic rock/metal templates. ✅ shipped as tuxguitar_generate_drums
- All generation lands behind the existing preview -> confirm -> undo flow.

## Phase 9 — Fingering, deeper

- Chord-aware optimization (state = set of simultaneous positions; search
  moves from DP to beam search / A*).
- Technique cost terms: alternate picking, sweep, legato friendliness.
- Genre presets for the CostModel (metal / jazz / classical weights).

## Phase 10 — Hardening & distribution

- Golden .tg test files (odd meters, tuplets, ties, repeats) with checked-in
  expected JSON on both sides.
- CI job building the Java plugin against the TuxGuitar 2.0.1 tag (cached),
  plus a 2.1.0 canary build.
- Signed/notarized macOS binary; plugin install script for Linux/Windows;
  build-from-source TuxGuitar dev environment (brew cask dies 2026-09-01).

## Idea backlog (unordered — pull into phases as they mature)

**Analysis & explanation**
- Repeated-riff detection (find the motif, name its variations, map song
  structure: intro/verse/chorus from repetition + markers)
- Difficulty estimation per section (tempo x fingering cost x technique
  density), "hardest 4 bars of this song" answers
- Chord detection from stacked notes + chord-name annotation writing
- Voice-leading analysis between chords; suggest smoother voicings
- Rhythm analysis: syncopation profile, accent map, groove fingerprint
- "Practice coach": given a target tempo, find passages whose fingering
  cost exceeds a playability threshold and propose simplifications

**Editing & tools**
- `simplify_selection` (drop notes, reduce stretches, keep the hook)
- Quantize / humanize timing and velocities
- Capo support (recompute frets under a capo, annotate)
- Section markers + song-structure tools (insert/rename markers, navigate)
- Multi-measure copy/paste/variation tools (write m5-8 as a variation of 1-4)
- Double-track tool: clone rhythm track with slight velocity/timing spread
- Lyrics read/write; text annotations on beats
- Tempo-map editing (gradual accelerando via per-measure tempo writes)

**Generation**
- Jam mode: "continue this riff for 8 bars in the same style"
- Arpeggiator: chord symbols -> picked patterns at chosen subdivision
- Riff variation engine (rhythmic displacement, octave jumps, pedal tones)
- Groove templates for drum generation (rock/metal/punk/blues presets)
- Bass humanization: approach notes, passing tones, octave pops

**Playback & practice**
- Play a specific measure range / the selection (needs caret positioning
  over the bridge)
- Loop a range at reduced tempo, step tempo up per repetition (TuxGuitar
  has loop + tempo-percent transport modes to drive)
- Metronome / count-in toggles over MCP

**Interop**
- Export via TuxGuitar's own writers: Guitar Pro, MusicXML, MIDI, PDF,
  LilyPond (trigger `action.song.write` with format + user-picked path)
- Import: open a file into a new tab over the bridge
- Headless mode: Rust-side .tg reader/writer so analysis tools work
  without TuxGuitar running (PLAN.md's original file-based vision)

**MCP surface**
- MCP resources: expose the score summary / selection as subscribable
  resources so clients keep live context without polling
- MCP prompts: canned workflows ("analyze this song", "make it easier",
  "write a solo over this progression")
- Elicitation for confirm steps (ask the user in-client instead of the
  two-call confirm dance, where clients support it)
- Streaming playback position notifications during play

**Virtual ear (AI feedback loop)** — v1 shipped as tuxguitar_analyze_arrangement
(symbolic cross-track listening: dissonance clashes, register masking,
rhythmic tightness, empty bars, velocity balance); v2 shipped as
tuxguitar_render_and_listen (headless MIDI -> fluidsynth + TuxGuitar's own
MagicSFver2 soundfont -> WAV -> DSP: loudness, clipping, spectral balance,
quiet holes; WAV kept for the user). v3 ideas: per-track stem rendering,
measure-aligned audio mapping, score-vs-audio onset verification.
- `render_and_analyze`: headless MIDI export -> fluidsynth render -> DSP
  feature extraction (onset alignment, per-track loudness curves, spectral
  balance, low-end mud detection) -> structured report the AI edits from.
  Claude can't hear audio directly, but for symbolic music the score +
  rendered-mix features cover nearly everything audible.

**Far future / research**
- Audio-to-tab sketching (hum a riff, get a draft tab)
- Style transfer ("re-voice this like a jazz comper / like doom metal")
- Multi-user jam: two MCP clients editing different tracks with the
  revision system arbitrating
