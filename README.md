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

Milestone 1 complete — the spine works end-to-end against TuxGuitar 2.0.1:

- Plugin loads, listens on loopback, publishes a token-protected discovery file
- `tabmcp doctor` connects, authenticates, and reads the open score
  (metadata, tracks, tunings, tempo/time-signature map)
- `tabmcp spike-test` applies a hard-coded edit **through TuxGuitar's undo
  stack** — Cmd+Z in the GUI reverts it; `undo`/`redo` over the wire work
- Bridge simulator (`tabmcp bridge-sim`) + integration tests run without TuxGuitar

Next: MCP `serve` (read tools), then the change-set edit model (see PLAN.md
phases).

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
