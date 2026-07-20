# TabMCP — an AI musician inside TuxGuitar

TabMCP connects MCP-compatible AI clients (Claude Code, Claude Desktop, ...)
to the score currently open in [TuxGuitar](https://tuxguitar.app). The AI can
**compose, arrange, listen to its own output, critique it, and revise** —
like a musician in the studio, not a one-shot generator.

- **Rust service** (`crates/`): the MCP server, normalized score model,
  music-theory engine (scales, chords, fingering, generation, critique),
  and the audio "ear" (render + DSP analysis).
- **Java plugin** (`tuxguitar-mcp-bridge/`): a thin bridge inside TuxGuitar
  exposing the live score over a localhost socket and applying edits through
  TuxGuitar's own action/undo system.

Docs: [PLAN.md](PLAN.md) (architecture) ·
[docs/PROTOCOL.md](docs/PROTOCOL.md) (bridge protocol) ·
[docs/ROADMAP.md](docs/ROADMAP.md) (what's next).

## The AI Ear refinement loop

The heart of the project: instead of assuming generated music sounds good,
the AI iterates —

1. **Compose** a riff / arrangement (or read what the user wrote)
2. **Evaluate** — `tuxguitar_evaluate` scores every track: groove
   consistency, motif repetition (with the recurring interval pattern),
   note density, robotic-dynamics detection, cross-track dissonance
   clashes, register masking, rhythmic tightness, key/scale
3. **Listen** — `tuxguitar_render_and_listen` renders the real mix through
   TuxGuitar's own soundfont (headless MIDI → fluidsynth → WAV → DSP:
   loudness, clipping, spectral balance, quiet holes);
   `tuxguitar_listen_stems` renders **each track in isolation** to hear
   which instrument causes a problem
4. **Fix** the top issue with the edit tools — every edit previews first,
   is revision-checked, and lands in TuxGuitar's undo stack
5. **Repeat**, narrating each pass — the undo stack is the version history
   (one Cmd+Z per pass)

## Tool surface (38 tools + 2 prompts)

| Area | Tools |
|---|---|
| Status & reading | `get_bridge_status`, `get_score_summary`, `get_measures`, `get_selection` |
| Analysis | `evaluate` (AI Ear scorecard), `analyze_arrangement`, `detect_key_and_scale` (44-scale catalog incl. phrygian dominant, hirajoshi, ...), `detect_chords`, `explain_selection` |
| Audio ear | `render_and_listen` (full mix + per-measure levels), `listen_stems` (per track, with auto-prescriptions) |
| Writing | `replace_measures` (chords, tuplets, two voices, pinch harmonics, bend curves), `transpose`, `humanize`, `copy_measures`, `import_midi` (MIDI -> optimized tab) |
| Fingering | `optimize_fingering` — chord-aware DP with explanations, fret-range constraints, cost presets (`metal`) |
| Generation | `generate_bassline` (root-anchor detection, soundfont-safe register), `generate_harmony` (3rds/6ths, any catalog scale), `generate_drums` (styles: rock, metal-gallop, punk, halftime, blast, d-beat; meter-aware; `target_track` for per-section grooves) |
| Structure | `create_track` (presets incl. 7-string A standard, bass clef, percussion), `change_tuning`, `set_tempo`, `set_time_signature` (odd meters), `set_key_signature`, `insert_measures`, `delete_measures`, `set_repeat` (loops), `set_marker` |
| Transport & practice | `play`, `play_from`, `stop`, `toggle_metronome`, `toggle_count_in` |
| Files | `save_copy`, `export` (multitrack MIDI, Guitar Pro, ...) |
| History | `undo`, `redo` |

**MCP prompts** (one-click workflows): `compose` (style/key/bars -> full
compose-and-refine session) and `refine` (N AI-Ear passes on the open score).

(All tool names carry the `tuxguitar_` prefix.)

Safety model: every mutating tool is **two-step** (preview -> confirm bound
to the previewed revision), **revision-checked** (stale writes rejected),
**atomic**, and **undoable** — including auto-appended measures and
generated tracks. A golden wire-format fixture in CI guards the
Rust<->Java protocol against accidental changes.

## Quickstart

Requirements: macOS with TuxGuitar 2.x installed, Rust toolchain, JDK 11+,
Maven; `brew install fluid-synth` for the audio ear.

```sh
# 1. Build + install the plugin (once per plugin update)
scripts/install-tuxguitar-deps.sh "/Applications/<your TuxGuitar>.app"   # once
( cd tuxguitar-mcp-bridge && mvn package )
cp tuxguitar-mcp-bridge/target/tuxguitar-mcp-bridge.jar \
   "/Applications/<your TuxGuitar>.app/Contents/MacOS/share/plugins/"

# 2. Install the MCP server binary
cargo install --path crates/tabmcp-server        # ~/.cargo/bin/tabmcp

# 3. Register with your MCP client (once — updates are picked up on restart)
claude mcp add tuxguitar -- ~/.cargo/bin/tabmcp serve
```

Start TuxGuitar, then in a fresh Claude session try:

> *"Write an 8-bar metal riff in E minor with a pinch harmonic, generate
> bass and drums from it, then refine it with the AI Ear loop until the
> scorecard is clean — and let me hear every pass."*

## Development

```sh
cargo test --workspace            # Rust suites incl. client<->simulator tests
( cd tuxguitar-mcp-bridge && mvn test )   # Java tests against real TG jars
scripts/dev-reload.sh             # rebuild plugin + restart TuxGuitar + wait
tabmcp doctor                     # connectivity + score summary
tabmcp bridge-sim                 # develop the Rust side without TuxGuitar
```

CI: GitHub Actions runs fmt/clippy/tests for Rust, and builds TuxGuitar
2.0.1 from source (cached) to compile and test the Java plugin.

## Security

Loopback-only socket; 32-byte random token in a 0600 discovery file
(`~/.tuxguitar-mcp/bridge.json`); no file paths or commands accepted over
the wire (exports go through TuxGuitar's own dialogs; renders use fixed
scratch paths under `~/.tuxguitar-mcp/`).

## License

- Rust workspace: MIT OR Apache-2.0
- `tuxguitar-mcp-bridge/`: LGPL-2.1 (links against TuxGuitar, which is LGPL-2.1)
