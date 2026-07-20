# TabMCP Bridge Protocol v1

Normative description of the wire protocol between the TabMCP Rust service
(client) and the TuxGuitar bridge plugin (server). The Rust structs in
`crates/tabmcp-model` are the reference implementation; the Java DTO mapping in
`tuxguitar-mcp-bridge` must match them byte-for-byte on the wire.

## Transport

- JSON-RPC 2.0, one message per newline-terminated UTF-8 line (NDJSON).
- Loopback TCP only. The plugin binds `127.0.0.1:0` and never another interface.
- One client at a time, served sequentially; a disconnect frees the slot.
- Idle sockets are closed by the plugin after 5 minutes; clients reconnect on demand.

## Discovery & authentication

While listening, the plugin maintains `~/.tuxguitar-mcp/bridge.json` (0600 on POSIX):

```json
{
  "protocolVersion": 1,
  "port": 57538,
  "token": "64 hex chars (32 random bytes)",
  "pid": 54940,
  "tuxguitarVersion": "2.0.1",
  "startedAtUnix": 1784535564
}
```

The file is deleted on plugin disconnect. A file whose `pid` is dead or whose
`port` refuses connections is stale and means "TuxGuitar not running".

The first request on a connection MUST be `hello` carrying the token. Every
other method before successful `hello` fails with `E_NOT_AUTHENTICATED`.

## Versioning & compatibility

- Single integer `protocolVersion`, negotiated in `hello`; mismatch →
  `E_PROTOCOL_VERSION` (the error message names the server's version).
- Unknown JSON fields are ignored by both sides. New optional fields do not
  bump the version; changed semantics do.

## Errors

JSON-RPC error object with `code: -32000`, human-readable `message`, and
`data.code` from the stable enum:

`E_NOT_AUTHENTICATED`, `E_PROTOCOL_VERSION`, `E_NO_DOCUMENT`,
`E_STALE_REVISION`, `E_INVALID_RANGE`, `E_UNSUPPORTED`, `E_EDIT_FAILED`,
`E_LOCKED`, `E_INTERNAL`.

## Revisions

The plugin maintains a monotonic `revision`, incremented on every
measure/song update event, and a `documentId` (UUID) rotated when a different
song is loaded. All reads return the current revision; Phase-4 writes carry
`expectedRevision` and fail with `E_STALE_REVISION` on mismatch.

## Conventions

- Time: ticks, 960 per quarter note (TuxGuitar's `QUARTER_TIME`); the first
  measure starts at tick 960.
- Tracks and measures are 1-based (as displayed in TuxGuitar).
- Strings are 1-based; string 1 is the highest-sounding string; `openPitch`
  is the MIDI pitch of the open string. Pitch is always derived:
  `pitch = openPitch + fret`.

## Methods (v1)

### hello
Params: `{ token, protocolVersion, clientInfo: { name, version } }`
Result: `{ protocolVersion, serverInfo: { tuxguitarVersion, pluginVersion }, capabilities: ["read","edit","undo"] }`

### ping
Result: `{ revision, documentOpen, playing }`

### read_song
Result:
```json
{
  "metadata": { "title": "", "artist": "", "album": "", "author": "", "comments": "" },
  "tracks": [{
    "number": 1, "name": "Track 1",
    "strings": [{ "number": 1, "openPitch": 64 }, ...],
    "program": 25, "isPercussion": false, "offset": 0,
    "maxFret": 24, "measureCount": 1
  }],
  "headers": [{
    "number": 1, "startTick": 960,
    "timeSignature": { "numerator": 4, "denominator": 4 },
    "tempoBpm": 120, "repeatOpen": false, "repeatClose": 0,
    "repeatAlternative": 0, "marker": "optional, omitted when absent"
  }],
  "revision": 1,
  "documentId": "uuid"
}
```
Errors: `E_NO_DOCUMENT`.

### spike_edit  *(Milestone 1 only — removed when `apply_changeset` lands)*
Hard-coded undoable edit at track 1 / measure 1 / beat 1: toggles string-6
fret 5↔7, or adds a fret-5 quarter note if none exists.
Result: `{ track, measure, description, newRevision }`
Errors: `E_NO_DOCUMENT`, `E_EDIT_FAILED`, `E_LOCKED`.

### undo / redo
Result: `{ performed, newRevision }` — `performed: false` when the stack is empty.

## Planned for Phase 3/4 (not yet implemented)

`read_measures`, `read_selection`, `apply_changeset` (with
`expectedRevision`), `save_copy`, `play`/`play_selection`/`stop`.
