# TabMCP — Improved Implementation Plan

**AI-native tablature assistant for TuxGuitar: Rust MCP server + thin Java bridge plugin.**

Plan version 1.0 — 2026-07-20. Grounded in Phase-0 inspection of the real TuxGuitar
source (github.com/helge17/tuxguitar, tag `2.0.1`, commit `533efa74`) and the locally
installed app (`/Applications/tuxguitar-2.0.1-macosx-swt-cocoa-x86_64.app`). All Java
class names in this document were verified in that source tree — none are guessed.

---

## 1. Executive Summary

TabMCP lets an MCP-compatible AI client read, analyze, and safely edit the score
currently open in TuxGuitar. A Rust process implements the MCP server, the normalized
score model, music-theory analysis, and change-set generation. A thin Java plugin
inside TuxGuitar exposes the live score over a localhost JSON-RPC socket and applies
structured edits through TuxGuitar's own action/undo system so every AI edit is
undoable with Ctrl+Z.

Key architectural decisions (details and rationale in later sections):

- **The Java plugin is the IPC server; the Rust process is the IPC client.** The MCP
  client (Claude, etc.) launches the Rust binary over stdio — the standard MCP
  transport — and the Rust binary connects to the plugin's loopback socket. Nobody
  launches anybody else's process; discovery happens via a token-protected port file.
- **JSON-RPC 2.0 over newline-delimited JSON on a loopback TCP socket.** TuxGuitar
  itself already uses localhost `ServerSocket` IPC (`TuxGuitar-synth` remote host), so
  this is an established pattern in the codebase. On this machine the point is
  decisive: TuxGuitar 2.0.1 runs as an **x86_64 JVM under Rosetta** while the Rust
  binary is native arm64 — JNI is not just undesirable, it is *impossible* across
  architectures. Sockets don't care.
- **Edits are measure-range replacements, not note-level patches.** TuxGuitar's model
  has no stable IDs, and its undo system snapshots measure ranges
  (`TGUndoableMeasureGenericController`). One transactional primitive —
  "replace the contents of measures M..N on track T, expecting revision R" — maps
  exactly onto what TuxGuitar can undo atomically, and every higher-level edit
  (transpose, simplify, harmonize) compiles down to it in Rust.
- **Optimistic concurrency via a bridge-maintained revision counter**, bumped on every
  `TGUpdateEvent`. Stale writes are rejected; the AI re-reads and retries.
- **MVP = read + explain + transpose-with-preview + undo + save-copy.** Generation,
  fingering optimization, and drums are explicitly deferred until the bridge and edit
  model are proven.

## 2. Recommended Architecture

```
AI client (Claude Desktop / Claude Code / any MCP client)
   │  MCP (stdio, JSON-RPC)                      launched by the AI client
   ▼
tabmcp (Rust binary)
   ├─ MCP tool layer            (tabmcp-server)
   ├─ theory & analysis         (tabmcp-theory)
   ├─ normalized score model    (tabmcp-model)
   └─ bridge client             (tabmcp-bridge)
   │  JSON-RPC 2.0 / NDJSON / TCP 127.0.0.1:<random port> + auth token
   ▼
tuxguitar-mcp-bridge.jar (Java plugin, in share/plugins)
   ├─ loopback socket server + discovery file
   ├─ DTO mapping (TG model ⇄ protocol JSON)
   └─ edit application via TGActionManager + TGUndoableManager
   ▼
TuxGuitar 2.0.1  (app.tuxguitar.* — TGSong, TGSongManager, TGEditorManager,
                  Caret/Selector, MidiPlayer, undo stack, UI refresh)
```

Everything is local. No cloud services, no network beyond 127.0.0.1.

Division of labor:

- **Rust owns intelligence**: normalization, analysis, transposition math, change-set
  construction, diffs/previews, validation, safety checks, MCP protocol.
- **Java owns TuxGuitar**: reading the live model, mapping to DTOs, applying
  change-sets inside the edit lock on the UI thread, undo registration, UI refresh,
  playback triggers, save actions. No music logic in Java beyond what TuxGuitar's
  managers already provide (e.g. `TGMeasureManager.autoCompleteSilences`).

The Rust core stays editor-independent: `tabmcp-model` and `tabmcp-theory` know
nothing about TuxGuitar, so a future file-based mode (MusicXML/GP) or another editor
bridge can reuse them unchanged.

## 3. Communication Mechanism Decision

**Chosen: JSON-RPC 2.0, newline-delimited JSON (NDJSON) frames, loopback-only TCP,
random ephemeral port, shared-secret token, discovery file.**

Evaluation of the required alternatives:

| Option | Verdict |
|---|---|
| JSON-RPC over localhost TCP | ✅ **Chosen.** Simple in both languages (plain `ServerSocket` / `tokio::net::TcpStream`), debuggable with `nc`, precedent inside TuxGuitar itself (`app.tuxguitar.midi.synth.remote.TGRemoteHost`). |
| JSON-RPC over WebSocket | ❌ Adds a handshake/framing layer and a Java dependency for zero benefit — no browser is involved. |
| JSON over stdin/stdout | ❌ Requires a parent↔child process relationship. The plugin would have to spawn and own the Rust process, but MCP clients *also* want to spawn the Rust process over stdio. Coupling launch order to IPC is the wrong shape here. |
| Unix domain sockets + Windows fallback | ❌ Two code paths where one suffices. Java's `UnixDomainSocketAddress` needs Java 16+ — fine on the bundled JRE 24, but not worth maintaining a Windows named-pipe fallback for a localhost-only link. |
| HTTP over localhost | ❌ Request/response only (server-push for future streaming gets awkward), needs an HTTP stack in the plugin, no real gain over raw JSON-RPC. |
| JNI | ❌ Ruled out categorically: architecture mismatch on this very machine (x86_64 JVM under Rosetta vs arm64 Rust), crash-domain sharing, build complexity. |

**Connection topology.** The plugin binds `127.0.0.1:0` (ephemeral port) on
`connect()` and writes a discovery file `~/.tuxguitar-mcp/bridge.json`
(`0600` permissions):

```json
{ "protocolVersion": 1, "port": 49213, "token": "<32 random bytes, base64>",
  "pid": 12345, "tuxguitarVersion": "2.0.1", "startedAt": "2026-07-20T10:00:00Z" }
```

The Rust binary reads the file, connects, and must authenticate with the token in the
`hello` request before any other method is accepted. The file is deleted on
`disconnect()`; a stale file (dead pid / connection refused) is treated as
"TuxGuitar not running" and reported through `get_bridge_status`, never as a crash.
The Rust side reconnects automatically with backoff, so TuxGuitar can be restarted
mid-session.

**Framing:** one JSON-RPC message per `\n`-terminated line, UTF-8, no length prefix.
A full score serializes to a few MB at worst — fine for line framing. Future
streaming = server-initiated JSON-RPC notifications on the same socket (the framing
already permits it; nothing to retrofit).

## 4. Rust Workspace Structure

The original plan's 9 crates are over-partitioned for an MVP (fingering, generation,
validation, client, protocol as separate crates before any of them exist). Four
crates carry their weight from day one; split later only when a boundary proves real:

```
tabmcp/
├── Cargo.toml                  # workspace
├── crates/
│   ├── tabmcp-model/           # normalized score model + protocol DTOs + change-sets
│   │                           #   (serde types shared by bridge & server; schema source of truth)
│   ├── tabmcp-theory/          # pure functions: pitch/interval math, transposition,
│   │                           #   scale/key/chord detection, explanation rendering
│   │                           #   (later: fingering, generation — as modules first)
│   ├── tabmcp-bridge/          # JSON-RPC client: discovery file, TCP, auth, retries,
│   │                           #   reconnect, typed method wrappers, mock bridge for tests
│   └── tabmcp-server/          # binary `tabmcp`: MCP server (stdio) wiring tools to
│                               #   bridge+theory; subcommands: `serve` (default),
│                               #   `doctor` (connectivity check), `bridge-sim` (mock
│                               #   TuxGuitar for integration tests)
├── tuxguitar-mcp-bridge/       # the Java plugin (Maven module, see §5)
├── testdata/                   # golden .tg / .gp5 files + expected JSON snapshots
└── docs/PROTOCOL.md            # bridge protocol v1, hand-maintained
```

Dropped from the original: `tabmcp-protocol` (lives in `tabmcp-model` — the DTOs *are*
the protocol), `tabmcp-analysis`/`tabmcp-fingering`/`tabmcp-generation` (modules inside
`tabmcp-theory` until they grow), `tabmcp-validation` (change-set validation belongs
with the change-set types in `tabmcp-model`), `tabmcp-cli`/`tabmcp-client` (subcommands
of the one binary; `tabmcp-bridge` is the client).

MCP implementation: the official Rust SDK (`rmcp`) over stdio. Tokio for async;
`serde`/`serde_json`; `tracing` for logs (to stderr — stdout belongs to MCP);
`thiserror` for error types. Minimal beyond that.

## 5. Java Plugin Structure

A standalone Maven project (not inside the TuxGuitar tree) compiled against TuxGuitar
2.0.1 artifacts installed to the local repo (`mvn install` once from the tag checkout;
artifacts are `app.tuxguitar:tuxguitar-lib:9.99-SNAPSHOT` etc. — the 9.99-SNAPSHOT
placeholder version is what the 2.0.1 tag actually builds). All TuxGuitar deps are
`provided`-scope; the jar drops into
`TuxGuitar.app/Contents/MacOS/share/plugins/`.

```
tuxguitar-mcp-bridge/
├── pom.xml                     # release 11; shades+relocates Gson (TuxGuitar has no
│                               #   JSON lib; relocation avoids TGClassLoader conflicts)
└── src/main/
    ├── java/app/tuxguitar/mcp/
    │   ├── TGMcpBridgePlugin.java     # implements TGPlugin (getModuleId/connect/disconnect)
    │   ├── server/                    # ServerSocket accept loop, NDJSON framing,
    │   │                              #   token auth, single-client policy, lifecycle
    │   ├── rpc/                       # JSON-RPC dispatch: method registry, request ids,
    │   │                              #   typed errors, timeouts
    │   ├── dto/                       # protocol DTOs (mirror tabmcp-model) + JSON codec
    │   ├── read/                      # TG model → DTO mapping; revision tracking
    │   │                              #   (TGUpdateEvent listener); caret/selection readers
    │   ├── edit/                      # change-set application: custom TGActions wrapped
    │   │                              #   in TGUndoableMeasureGenericController, executed
    │   │                              #   via TGActionManager under TGEditorManager lock
    │   └── ui/                        # status entry in Tools menu (TGToolItemPlugin
    │                                  #   pattern from TuxGuitar-tuner): connected state,
    │                                  #   versions, last error, restart-listener action
    └── resources/META-INF/services/app.tuxguitar.util.plugin.TGPlugin
```

Rules the plugin follows:

- All model reads/writes go through `TGEditorManager.runLocked(...)`; anything that
  must touch UI or fire actions goes through `TGSynchronizer.executeLater(...)` or the
  sync-thread action interceptor — the socket thread never touches the model directly.
- DTOs never leak TuxGuitar types; TuxGuitar types never cross the wire.
- All state creation via `TGFactory` (model classes are abstract — verified).
- Disconnections are normal, not errors: the accept loop just waits for the next client.

## 6. Shared Protocol Design

**docs/PROTOCOL.md is the source of truth**, with the Rust structs in `tabmcp-model`
as the reference implementation and mirrored Java DTOs. Schema-generation (JSON Schema
→ both languages) is *deferred*: for one team and ~15 methods, integration tests
asserting byte-level JSON compatibility (same golden files deserialized on both sides)
catch drift with far less machinery. Revisit if the protocol passes ~40 methods.

Envelope: JSON-RPC 2.0 (`id`, `method`, `params`, `result`, `error`). Conventions:

- **Handshake:** first request must be
  `hello { token, protocolVersion, clientInfo }` →
  `{ serverInfo: {tuxguitarVersion, pluginVersion}, protocolVersion, capabilities: ["read","edit","undo","playback","save"] }`.
  Version policy: single integer; server accepts equal versions, else returns
  `E_PROTOCOL_VERSION` with its own version so the Rust side can print a useful
  message. Capabilities let a future Android/older bridge advertise subsets.
- **Health:** `ping` → `{ revision, documentOpen, playing }` (doubles as revision poll).
- **Errors:** JSON-RPC error with `code` from a stable enum
  (`E_NOT_AUTHENTICATED`, `E_NO_DOCUMENT`, `E_STALE_REVISION`, `E_INVALID_RANGE`,
  `E_UNSUPPORTED`, `E_EDIT_FAILED`, `E_LOCKED`, `E_INTERNAL`), human `message`, and
  structured `data` (e.g. `{expected: 41, actual: 44}` for stale revisions).
- **Timeouts:** Rust side, per-method class — 2 s for reads/ping, 10 s for edits
  (edits queue behind the UI thread). The plugin never blocks the socket thread on UI
  work without a deadline.
- **Backward compatibility:** unknown JSON fields are ignored on both sides
  (serde `deny_unknown_fields` OFF for wire types); new optional fields never bump the
  version; changed semantics do.

Bridge method set v1 (plugin-side; deliberately smaller than the MCP tool list —
several MCP tools are Rust-side computations over `read_*` results):

```
hello, ping
read_song            → metadata, tracks (name, tuning, channel, isPercussion,
                       measureCount), tempo/time-signature map, revision
read_measures        { trackNumber, from, to } → beats/voices/notes/effects, revision
read_selection       → { trackNumber, from, to, caret } | null, revision
apply_changeset      { expectedRevision, changes: [...] } → { newRevision, summary }
undo, redo
save_copy            { suggestedName } → { path }   (Save-As dialog or sibling file —
                       decided in Phase 4; no arbitrary paths from the wire)
play { from, to } | play_selection, stop
```

## 7. Internal Score Model (Rust, `tabmcp-model`)

Normalized, editor-independent, `serde`-serializable. Time is measured in **ticks with
960 per quarter note** — TuxGuitar's own base (`TGDuration.QUARTER_TIME = 960`), which
makes round-tripping exact.

MVP fields (all required for correct round-trip of the golden files):

```
Score      { metadata{title, artist, album, author, comments…}, tracks[], headers[], revision }
Header     { number, startTick, timeSignature{num, den}, tempoBpm, repeatOpen,
             repeatClose, repeatAlternative, marker? }
Track      { number, name, strings[{number, openPitch}], channel{program, isPercussion},
             offset, maxFret, color? }
Measure    { headerNumber, clef, keySignature, beats[] }
Beat       { startTick, voices[2] }
Voice      { duration{value, dotted, doubleDotted, tuplet{enters, times}}, notes[],
             isRest, direction }
Note       { string, fret, velocity, tied, effects }
Effects    { bend?, slide, hammer, vibrato, palmMute, letRing, ghost, dead, accent,
             heavyAccent, staccato, harmonic?, grace?, trill?, tremoloPicking?,
             tapping, slapping, popping, fadeIn }
```

Design rules:

- **Pitch is derived, never stored**: `pitch = track.strings[note.string].openPitch + note.fret`
  (+12 for natural harmonics etc. later). String+fret is the source of truth because
  this is tablature, not MIDI — exactly the original plan's intent.
- **Lossless-by-scope**: the model mirrors TuxGuitar's own structures 1:1 for
  everything listed, so nothing in scope is destroyed. Out-of-scope structures
  (lyrics, chord diagrams, stroke/pickStroke, tremolo bar, per-measure lineBreak) are
  **not** round-tripped through Rust in the MVP — instead they are *preserved by
  construction*: the measure-replacement edit rebuilds only beats/voices/notes, and
  the Java side copies untouched aspects from the existing measure. `read_measures`
  flags such content (`hasUnsupported: ["lyrics"]`) so Rust can warn before replacing.
- Deliberately absent (TuxGuitar has no such concept — the original plan listed them
  speculatively): per-note IDs, "Instrument" beyond MIDI channel, standalone "Chord"
  entities (a chord is just multiple notes in one voice; `TGChord` diagrams are
  out-of-scope metadata), free-floating "Rest" (a rest is an empty voice with a
  duration, as in TuxGuitar).

## 8. Edit and Revision Model

**Revision:** the plugin holds an `AtomicLong` revision, incremented on every
`TGUpdateEvent` of type `MEASURE_UPDATED`, `SONG_UPDATED`, or `SONG_LOADED`
(registered via `TGEditorManager.addUpdateListener`). Every read response carries it;
every `apply_changeset` carries `expectedRevision` and fails with `E_STALE_REVISION`
if it doesn't match at apply time *inside the edit lock*. `SONG_LOADED` additionally
rotates a document UUID so a revision from one song can never validate against another.

**Change-set:** an ordered list of operations, applied atomically (all inside one
`runLocked` + one undoable edit) or not at all:

```
ReplaceMeasureRange { trackNumber, from, to, measures: [Measure] }   // the workhorse
CreateTrack         { name, strings[], program, afterTrack? }
ChangeTuning        { trackNumber, strings[], transposeStrategy: Keep|Shift }
SetTempo            { headerNumber, bpm }                            // post-MVP
InsertMeasures / DeleteMeasures { at, count }                        // post-MVP
```

The original plan's note-level operations (insert/delete/update note, replace beat)
are intentionally **not** wire operations: without stable IDs they'd need fragile
beat-tick addressing and per-op conflict rules, and TuxGuitar's undo controllers
snapshot measure ranges anyway. Rust performs note-level edits on its normalized
copy, then ships the resulting measures. Wire cost of a few measures is trivial;
semantics become "compare-and-swap on a measure range", which is easy to reason
about, easy to undo, and easy to test.

**Undo — the load-bearing detail** (verified in source): undo in TuxGuitar is *not*
automatic. The app maps action ids to `TGUndoableActionController`s in
`TGActionConfigMap`; a pre/post `TGUndoableActionListener` snapshots state around
execution and pushes a `TGUndoableEdit` into `TGUndoableManager`. Raw `TGSongManager`
mutations bypass undo entirely. Therefore the plugin registers its **own** action
(`action.mcp.apply-changeset`) via `TGActionManager.mapAction(...)`, adds it to the
sync-thread interceptor (the exact recipe `TGToolItemPlugin.connect()` uses), and
inside `processAction` wraps the mutation in
`TGUndoableMeasureGenericController.startUndoable(...)` / `endUndoable(...)` +
`TGUndoableManager.addEdit(...)`, then calls
`TGEditorManager.updateMeasures(...)` + `redraw()`. Result: one Ctrl+Z reverts one AI
change-set. This is the single riskiest integration point and is validated by a
dedicated spike in Phase 2 before anything is built on top of it.

**Preview/dry-run:** `dry_run: true` is the *default* on every mutating MCP tool.
Rust computes the change-set locally and returns a human-readable diff (the format
from the original plan: track, measure range, notes replaced/added, what's preserved)
plus a `changeset_token`. Applying requires a second call with `confirm: true` and
the token; the token embeds the revision it was computed against, so a preview can
never be applied over changed state. Max edit range enforced in Rust (default 32
measures per change-set, configurable) as a blast-radius cap.

## 9. Final MVP Scope

Everything below, nothing more:

1. Java plugin: lifecycle, socket server, discovery file, token auth, status menu
   entry (connected/version/last error/restart).
2. Rust `tabmcp` binary: MCP over stdio, bridge client with reconnect, `doctor` and
   `bridge-sim` subcommands.
3. Handshake with protocol version + capabilities.
4. Reads: `read_song`, `read_measures`, `read_selection` → normalized model.
5. Revision tracking + stale-write rejection.
6. Edit path: `ReplaceMeasureRange` + `apply_changeset`, atomic, undoable, UI-refreshing.
7. Transposition in Rust (string/fret-aware: re-fret on same string where possible,
   report notes that fall off the fretboard instead of silently clamping).
8. Analysis in Rust: pitch classes, intervals, scale matching against a standard
   scale catalog, tonal-center heuristic (weighted pitch-class histogram + cadence
   bias), human-readable `explain_selection`.
9. Preview (default) / confirm flow with diff summaries.
10. Undo/redo via bridge; save-copy.
11. MCP tools (13): `get_bridge_status`, `get_score_summary`, `get_track`,
    `get_measures`, `get_selection`, `explain_selection`, `detect_key_and_scale`,
    `transpose_selection`, `apply_changeset`, `undo`, `redo`, `save_copy`,
    `play_selection`/`stop` (one tool, `action: play|stop`).
    Per-tool spec table (purpose, input/output schema, errors, read-only flag,
    selection-capable flag, confirmation requirement, expected latency) lives in
    docs/TOOLS.md and is written in Phase 3 — not duplicated here.
12. Golden test files + Rust/Java integration test suite.

**End-to-end acceptance use cases** (unchanged from original): ① read selection →
transpose +2 → preview → confirm → applied, UI updated, Ctrl+Z reverts. ② select a
riff → `explain_selection` returns notes, intervals, likely scale, tonal center.

## 10. Deferred Features

Post-MVP, roughly in order: fingering cost model & optimization (DP over per-note
string/fret candidates — algorithm evaluation happens then, not now) → simplify /
"make easier" → harmony track generation → bass generation → rhythm variation → drum
accents → repeated-riff detection → difficulty estimation → tempo/measure-structure
edits → track create/rename/delete tools → multi-document awareness beyond "current
tab" → export formats → schema-generated DTOs → streaming playback position → any
chat UI. Cloud anything: never in scope.

## 11. Implementation Phases

Each phase ends buildable and tested. Exact commands documented per phase in the repo.

- **Phase 0 — done** (this document + `docs/RESEARCH.md` appendix): repo inspected at
  tag 2.0.1, APIs verified, licensing read.
- **Phase 1 — Protocol skeleton (Rust-only)**: `tabmcp-model` wire types,
  `bridge-sim` mock bridge, `tabmcp-bridge` client with discovery/auth/reconnect,
  `doctor` green against the sim. Serialization + error + reconnect tests.
- **Phase 2 — Plugin spike ("hello, undoable world")**: plugin loads in the installed
  2.0.1 app, socket + handshake work end-to-end, and a **hard-coded** measure edit
  goes through the custom-action path and is undoable with Ctrl+Z. This retires the
  #1 risk before any breadth is built. Also: revision listener, status menu entry.
- **Phase 3 — Read path**: DTO mapping (song/measures/selection incl. `Selector`
  beat-range), MCP read tools + `explain_selection`/`detect_key_and_scale` in Rust,
  golden-file snapshot tests both sides.
- **Phase 4 — Safe edit path**: change-set model, transposition, preview/confirm
  tokens, stale rejection, `apply_changeset`, save-copy, undo/redo tools.
  → **MVP acceptance run.**
- **Phase 5 — Analysis depth**: chord detection, better tonal-center detection,
  richer explanations. **Phase 6 — Fingering. Phase 7 — Generation** (as deferred
  list; each gets its own mini-plan when reached).

## 12. Testing Strategy

- **Rust unit**: pitch/interval/transposition math (property tests: transpose +n then
  −n is identity where in-range), scale matching against known riffs, change-set
  validation, wire-type round-trips.
- **Java unit**: DTO mapping on `TGFactory`-built model objects (tuxguitar-lib is a
  plain jar — no UI needed), revision bookkeeping, JSON codec.
- **Cross-language golden tests**: `testdata/*.tg` (standard 6-string, 7-string A
  standard, multi-track, two voices, 5/4 + 7/8, tempo changes, ties, triplets, palm
  mutes, slides, bends, harmonics, repeats — authored once in TuxGuitar) with checked-in
  expected JSON; Java serializes → Rust deserializes → byte-compare, catching protocol
  drift without schema machinery.
- **Integration (scripted)**: `tabmcp` against `bridge-sim` for protocol/reconnect/
  stale-revision/large-score cases in CI; against real TuxGuitar (manual + a small
  checklist) for undo, UI refresh, selection, save-copy.
- **The acceptance use cases** (§9) as the release gate for the MVP tag.

## 13. Security Considerations

- Loopback bind only; single authenticated client; 32-byte random token in a `0600`
  file in the user's home — same trust boundary as the user's own session.
- No filesystem paths accepted over the wire (save-copy writes next to the original
  or via TuxGuitar's own dialog); no command execution; no reflection-based dispatch
  (static method registry).
- Read tools and write tools are disjoint; writes demand revision + confirm token;
  blast-radius cap on edit ranges.
- Payload logging off by default; logs carry request ids and edit summaries, not
  score contents.
- The MCP layer inherits the client's user consent flow for tool calls; destructive
  tools (`apply_changeset` with confirm) are additionally annotated non-read-only so
  clients prompt.

## 14. Packaging Strategy

- **Rust**: single static binary per platform (arm64/x86_64 macOS, Linux, Windows),
  distributed via GitHub releases + `cargo install`. Registered in the MCP client as
  `command: tabmcp` — the standard MCP install story. macOS Gatekeeper: document
  `xattr -d com.apple.quarantine` / build-from-source now; sign+notarize later.
- **Java**: one shaded jar; install = copy into the app's `share/plugins/` (macOS
  path documented; Linux/Windows paths in an install script later). Version pin:
  plugin states supported TuxGuitar versions (2.0.x initially) and refuses politely
  on mismatch via the handshake.
- **Dev-environment note (this machine)**: the brew cask is x86_64-under-Rosetta and
  deprecated (Gatekeeper; disabled 2026-09-01). It works for development — the plugin
  jar is architecture-independent and the bundled JRE is OpenJDK 24 — but the durable
  path is building TuxGuitar 2.0.1 from source natively (`mvn` per INSTALL.md,
  JDK 17+); plan for that switch when the cask dies. The Rust binary stays native
  arm64 either way — another socket-IPC dividend.
- No plugin-launches-Rust or Rust-launches-TuxGuitar: three independent lifecycles
  (editor, MCP server, AI client) connected by discovery, resilient to any restart
  order.

## 15. Licensing Risks

Verified: TuxGuitar is **LGPL-2.1** (`docs/LICENSE` in-tree). Assessment (not legal
advice; uncertainty flagged):

- **Java plugin**: links directly against tuxguitar-lib/app internals. Whether a
  plugin is a "work based on the Library" under LGPL-2.1 is the classic gray area;
  the safe, friction-free choice is to license the plugin **LGPL-2.1** itself. Cost
  is negligible; risk drops to zero. **Recommended.**
- **Rust service**: separate process, communicates over a socket, contains no
  TuxGuitar code → not a derivative work by any mainstream reading. License
  **MIT OR Apache-2.0**. Shipping both in one bundle ("mere aggregation") does not
  change either license's obligations, though a combined installer should include
  both license texts.
- **Uncertainty to document**: no case law directly on plugin-linking under LGPL 2.1;
  if the project ever vendors TuxGuitar code into the Rust side (e.g. porting the .tg
  parser), that code carries LGPL with it — avoid, or isolate.

## 16. Main Technical Risks

1. **Undo integration** — the whole product promise ("Ctrl+Z reverts the AI") hangs
   on the custom-action + `TGUndoableMeasureGenericController` path working from a
   plugin. Mitigation: it's the Phase 2 spike, before breadth. Fallback if generic
   controllers misbehave for plugin actions: implement `TGUndoableEdit` directly with
   our own before/after measure snapshots (interface verified: `undo/redo/canUndo/canRedo`).
2. **Threading/locking mistakes** — socket thread vs UI thread vs edit lock.
   Mitigation: one narrow choke point (`edit/` package) owns all mutation; reads copy
   under `runLocked` and serialize outside it.
3. **API drift 2.0.x → 2.1.0** — plugin pins 2.0.1 artifacts; a CI job compiles it
   against the 2.1.0 tag when released to surface breaks early. The `org.herac` →
   `app.tuxguitar` rename shows the project does break APIs at majors.
4. **Selection model coupling** — `Selector`/`Caret` live in the desktop app jar, not
   the lib; JFX/Qt toolkits or future refactors could move them. Contained in `read/`,
   feature-flagged by the `read_selection` capability.
5. **Fidelity gaps on replace** (losing lyrics/chord diagrams in replaced measures) —
   mitigated by preserve-by-construction (§7) + `hasUnsupported` warnings + preview.
6. **Rosetta/cask EOL** on the dev machine — §14; zero impact on the architecture.
7. **Tuplet/precise-time edge cases** (`preciseStart`, `TGDivisionType`) — golden
   files include triplets from day one; `autoCompleteSilences` after edits keeps
   measures well-formed.

## 17. Acceptance Criteria (MVP)

1. Fresh machine, documented steps only: install plugin jar + configure MCP client →
   `get_bridge_status` reports connected, with versions.
2. TuxGuitar closed: every tool degrades to a clear "bridge unavailable" message;
   Rust process neither crashes nor hangs; recovery is automatic when TuxGuitar starts.
3. Use case ①: transpose selection +2 via AI with preview → apply → correct frets in
   UI → single Ctrl+Z fully reverts → redo re-applies.
4. Use case ②: `explain_selection` on a known riff names the correct notes, interval
   structure, plausible scale and tonal center (golden-file assertions).
5. Editing the score in TuxGuitar between preview and confirm → apply is rejected
   with a stale-revision error, no partial edit occurs.
6. A change-set touching a measure with a bend/harmonic/triplet round-trips without
   corrupting those effects (golden files).
7. `save_copy` produces a loadable .tg; original document unmodified on disk.
8. All Rust tests + Java tests + cross-language golden suite green in CI (Linux);
   manual checklist passed on macOS against the installed app.
9. Clippy-clean, rustfmt-clean; plugin follows TuxGuitar code conventions.

## 18. Concrete First Implementation Milestone

**Milestone M1 — "the wire is real" (Phases 1+2, ~the next work session):**

1. `cargo new` workspace; `tabmcp-model` with `hello`/`ping`/`read_song` wire types.
2. `tabmcp bridge-sim` serving a canned song; `tabmcp doctor` connects through
   discovery file + token and prints the song summary. CI runs this as a test.
3. Maven module `tuxguitar-mcp-bridge`; `mvn install` of the 2.0.1 checkout
   documented; plugin loads in the installed app (visible in plugin list, status
   entry in Tools menu).
4. Plugin serves real `hello`/`ping`/`read_song` from the open document.
5. **Spike:** `action.mcp.apply-changeset` hard-codes "set fret 5 → 7 in measure 1"
   through the undoable path; demonstrated: edit appears, UI refreshes, Ctrl+Z
   reverts. Findings written into docs/RESEARCH.md.

Exit criterion: `tabmcp doctor` against *real* TuxGuitar prints the open song's
tracks and tunings, and the spike edit is undoable. Everything after this is
incremental breadth on a proven spine.

---

## Changes Made to the Original Plan

1. **Inverted the IPC topology** (plugin = server, Rust = client, MCP client launches
   Rust): the original left launch ownership open ("evaluate whether the plugin
   should launch the Rust process"). Discovery-file decoupling gives independent
   lifecycles and matches how MCP clients actually spawn stdio servers. It also
   dissolves the distribution question — no process supervision code anywhere.
2. **Ruled out JNI with evidence, chose TCP with precedent**: the installed TuxGuitar
   is an x86_64 JVM under Rosetta on an arm64 Mac (verified), making in-process
   binding impossible; TuxGuitar itself already ships localhost-socket IPC
   (`TuxGuitar-synth`), so TCP+NDJSON is the native pattern, not an import.
3. **Replaced note-level edit ops with one transactional primitive**
   (`ReplaceMeasureRange` + compare-and-swap revision): TuxGuitar has no stable
   note/beat/measure IDs (verified), so the original's note-ID-based ops were
   unimplementable as specified; measure-range replacement matches TuxGuitar's own
   undo granularity (`TGUndoableMeasureGenericController`). Note-level editing still
   exists — in Rust, on the normalized copy.
4. **Made undo integration a named, front-loaded risk with a concrete mechanism**:
   research showed undo is *not* automatic (action-id → controller mapping in
   `TGActionConfigMap`; raw manager mutations bypass it). The original plan assumed
   "integrate with undo" was an API call; it's actually the hardest part, so it
   became the Phase 2 spike with a verified fallback (`TGUndoableEdit`).
5. **Cut the Rust workspace from 9 crates to 4**: protocol merged into model,
   analysis/fingering/generation start as modules, CLI and client merged into the
   binary/bridge. Split-on-proven-need beats speculative partitioning.
6. **Trimmed the MCP surface from ~50 tools to 13** and split "bridge methods" from
   "MCP tools": many original tools (get_tempo_map, analyze_intervals, …) are
   Rust-side computations over three read methods, not bridge calls. Small surface →
   reliable v1, per the original's own advice.
7. **Made dry-run the default and previews token-bound to a revision**: the original
   had preview and revision checks as separate features; binding the confirm token to
   the revision it previewed closes the preview-then-race gap.
8. **Defined pitch as derived from string+fret, dropped speculative model entities**
   (standalone Rest/Chord/Instrument/note-IDs) to mirror TuxGuitar's real model
   (rest = empty voice, chord = multi-note voice) — eliminating a whole class of
   mapping bugs.
9. **Replaced schema-codegen with cross-language golden tests** for protocol-drift
   protection at MVP scale; codegen deferred until the protocol is large enough to
   pay for it.
10. **Grounded everything in verified 2.0.1 APIs** — including the critical discovery
    that all packages renamed to `app.tuxguitar.*` in 2.x (any `org.herac` reference
    found in old docs/tutorials is dead) — and pinned the plugin to the installed
    2.0.1, with a CI canary against 2.1.0.
11. **Added environment-specific packaging facts**: brew cask is deprecated and dies
    2026-09-01; documented the build-from-source successor path now instead of
    discovering it as an outage later.
12. **Resolved the licensing question concretely**: LGPL-2.1 verified in-tree; plugin
    → LGPL-2.1 (safe harbor), Rust service → MIT/Apache-2.0 (separate process, no
    linking), with the remaining gray area explicitly documented.

---

## Appendix: Verified TuxGuitar 2.0.1 API Reference (Phase-0 findings)

Source: github.com/helge17/tuxguitar @ tag `2.0.1` (commit `533efa74`); clone kept in
the session scratchpad. Packages: `app.tuxguitar.*` (Maven groupId `app.tuxguitar`,
artifact version `9.99-SNAPSHOT` placeholder). Build: Maven multi-module, JDK 17+ to
build, compiled at release 9; UI via `TuxGuitar-ui-toolkit` (SWT backend on macOS).

- **Plugin API**: `app.tuxguitar.util.plugin.TGPlugin` — `getModuleId()`,
  `connect(TGContext)`, `disconnect(TGContext)`; discovery via
  `META-INF/services/app.tuxguitar.util.plugin.TGPlugin` (custom `TGServiceReader`);
  jars loaded from `share/plugins` by `TGFileUtils.loadClasspath()` into a shared
  `TGClassLoader`. Menu-tool recipe: `app.tuxguitar.app.tools.custom.TGToolItemPlugin`
  (see `TuxGuitar-tuner`).
- **Model** (`app.tuxguitar.song.models`, abstract, built via
  `app.tuxguitar.song.factory.TGFactory`): `TGSong`, `TGTrack` (number, name,
  strings, channelId, offset, maxFret), `TGMeasureHeader` (number, start,
  timeSignature, tempo, repeat*, marker), `TGMeasure` (header, track, clef,
  keySignature, beats), `TGBeat` (start, voices[2], MAX_VOICES=2), `TGVoice`
  (duration, notes, index, direction), `TGNote` (value=fret, string, velocity,
  tiedNote, effect), `TGString` (number, value=open pitch), `TGDuration`
  (QUARTER_TIME=960, divisionType), `TGNoteEffect` (+`effects/` bend, harmonic,
  grace, trill, tremolo picking, tremolo bar), `TGChannel` (percussion channel 9).
- **Managers**: `TGSongManager` (`addTrack`, `changeTimeSignature`, `changeTempos`,
  `addNewMeasure`…), `TGMeasureManager` (`addNote(measure, start, note, duration,
  voice)`, `removeNote`, `getBeat(measure, start)`, `autoCompleteSilences`,
  `moveSemitoneUp/Down`…), `TGTrackManager`.
- **Actions**: `TGActionManager.getInstance(ctx)` — `mapAction(id, action)`,
  `execute(id, actionContext)`; helper `TGActionProcessor` (editor-utils);
  attribute keys in `TGDocumentContextAttributes`. Real ids incl.
  `action.tools.transpose-notes`, `action.track.change-tuning`,
  `action.transport.play`/`.stop`, `action.file.save-as`, `action.edit.undo`/`.redo`.
- **Undo**: per-action controllers wired in app-side `TGActionConfigMap`;
  `TGUndoableActionListener` (pre/post) + `TGUndoableManager.addEdit(...)`; generic
  controllers `TGUndoableMeasureGenericController` / `...Song...` / `...Track...` /
  `TGUndoableNoteRangeController` (editor-utils); raw manager mutations bypass undo;
  bypass flag `TGUndoableEditBase.ATTRIBUTE_BY_PASS_UNDOABLE`.
- **Threading/locking**: `TGSynchronizer.getInstance(ctx).executeLater(...)` (UI
  thread); `TGEditorManager` — `runLocked(Runnable)`, `updateMeasures(List<Integer>)`,
  `redraw()`, `addUpdateListener(...)`; events via `TGEventManager`, song-change
  signal `TGUpdateEvent` (`SELECTION=1, MEASURE_UPDATED=2, SONG_UPDATED=3,
  SONG_LOADED=4, SONG_SAVED=5`).
- **Document/selection/playback**: `TGDocumentManager.getInstance(ctx).getSong()`;
  multi-tab `TGDocumentListManager.findCurrentDocument()`; caret
  `TuxGuitar.getInstance().getTablatureEditor().getTablature().getCaret()`
  (track/measure/position/selectedBeat/string); selection
  `...getTablature().getSelector()` → `getBeatRange()`/`getNoteRange(voices)`
  (`TGBeatRange`, `TGNoteRange`); playback `MidiPlayer.play/pause/stop` or transport
  action ids; save via `action.file.save-as` / `TGFileFormatManager.write(...)`.
- **IPC precedent**: `desktop/TuxGuitar-synth/.../remote/TGRemoteHost.java` runs a
  localhost `ServerSocket` for out-of-process synth clients.
- **Installed app** (this machine): brew cask 2.0.1, x86_64 under Rosetta, bundled
  OpenJDK **24** at `Contents/MacOS/jre`, plugins at `Contents/MacOS/share/plugins/`
  (28 stock plugin jars). Cask deprecated, disabled 2026-09-01 (Gatekeeper).

**Still requiring verification during implementation** (flagged, not assumed):
exact `-Dtuxguitar.class.path` property name; Windows/Linux packaged plugin paths;
`TGSyncProcessLocked` helper semantics; whether generic undoable controllers work
unmodified for plugin-registered action ids (the Phase 2 spike answers this).
