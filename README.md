# TabMCP — AI-native tablature assistant for TuxGuitar

An MCP integration that lets AI clients read, analyze, and safely edit the
score currently open in [TuxGuitar](https://tuxguitar.app):

- **Rust service** (`crates/`): MCP server, normalized score model, music
  theory, change-set generation.
- **Java plugin** (`tuxguitar-mcp-bridge/`): thin bridge inside TuxGuitar
  exposing the live score over a localhost socket, applying edits through
  TuxGuitar's own undo system.

See [PLAN.md](PLAN.md) for the full architecture and
[docs/PROTOCOL.md](docs/PROTOCOL.md) for the bridge protocol.

## Status

Phase 8 complete — AI clients can read, analyze, write, refine, **and compose** in the open score
in TuxGuitar 2.0.1:

- **`tabmcp serve`** exposes 20 MCP tools over stdio — reading
  (`tuxguitar_get_bridge_status`, `tuxguitar_get_score_summary`,
  `tuxguitar_get_measures`, `tuxguitar_get_selection`), analysis
  (`tuxguitar_detect_key_and_scale`, `tuxguitar_explain_selection`), and
  editing (`tuxguitar_replace_measures`, `tuxguitar_transpose`,
  `tuxguitar_optimize_fingering`, `tuxguitar_generate_bassline`,
  `tuxguitar_generate_harmony`, `tuxguitar_generate_drums`,
  `tuxguitar_create_track`,
  `tuxguitar_change_tuning`, `tuxguitar_play`/`tuxguitar_stop`,
  `tuxguitar_undo`, `tuxguitar_redo`, `tuxguitar_save_copy`)
- Every edit is **two-step** (preview → confirm with the previewed revision),
  **revision-checked** (rejected if the score changed in between), **atomic**,
  and **undoable with a single Cmd+Z** — including auto-appended measures
- The bridge plugin (0.4.0) applies change-sets through TuxGuitar's undo
  system on the UI thread under the editor lock
- The theory engine detects scales/tonal centers and produces plain-language
  explanations; transposition is string/fret-aware and refuses edits that
  would fall off the fretboard
- **Fingering optimizer**: models the passage as a path-search problem
  (per-note string/fret candidates -> transition costs -> dynamic
  programming), with a swappable cost model (fret movement, position
  shifts, string skips, open-string preference, optional user fret range)
  and *explanations* — every result says WHY the chosen fingering is
  easier (e.g. "81% less hand effort; 2 position shifts eliminated;
  uses 3 open strings")

## Using with an MCP client

Install the binary and register it:

```sh
cargo install --path crates/tabmcp-server   # installs ~/.cargo/bin/tabmcp

# Claude Code:
claude mcp add tuxguitar -- ~/.cargo/bin/tabmcp serve
```

Then, with TuxGuitar running, ask the AI things like "what's open in
TuxGuitar?", "explain the riff I selected", or "what scale is this?".

## Building

### Rust (`tabmcp` binary)

```sh
cargo build --release          # binary at target/release/tabmcp
cargo test --workspace         # includes client<->simulator integration tests
```

### Java plugin

Requires JDK 11+ and Maven, plus an installed TuxGuitar 2.0.1 to compile against:

```sh
# once: install TuxGuitar's jars into the local Maven repo
scripts/install-tuxguitar-deps.sh /Applications/tuxguitar-2.0.1-macosx-swt-cocoa-x86_64.app

cd tuxguitar-mcp-bridge
mvn package                    # target/tuxguitar-mcp-bridge.jar
```

### Install the plugin

Copy the jar into TuxGuitar's plugin directory and restart TuxGuitar:

```sh
cp tuxguitar-mcp-bridge/target/tuxguitar-mcp-bridge.jar \
   "/Applications/tuxguitar-2.0.1-macosx-swt-cocoa-x86_64.app/Contents/MacOS/share/plugins/"
```

TuxGuitar's Tools menu gains two entries: **TabMCP: Bridge Status** and
**TabMCP: Spike Edit (undoable test)**.

## Trying it

With TuxGuitar running:

```sh
tabmcp doctor        # connect, authenticate, print the open score's summary
tabmcp spike-test    # apply an undoable test edit, then undo + redo it
```

Without TuxGuitar:

```sh
tabmcp bridge-sim    # simulated bridge with a canned song (Ctrl+C to stop)
tabmcp doctor        # in another terminal — same protocol, no TuxGuitar
```

## Security

Loopback-only socket, 32-byte random token in a 0600 discovery file, no
file paths or commands accepted over the wire. AI edits are revision-checked
and land in TuxGuitar's undo stack.

## License

- Rust workspace: MIT OR Apache-2.0
- `tuxguitar-mcp-bridge/`: LGPL-2.1 (links against TuxGuitar, which is LGPL-2.1)
