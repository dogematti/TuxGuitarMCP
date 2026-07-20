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

### read_measures
Params: `{ trackNumber, fromMeasure, toMeasure }` (1-based, inclusive)
Result:
```json
{
  "trackNumber": 1, "fromMeasure": 1, "toMeasure": 2,
  "measures": [{
    "number": 1, "keySignature": 0,
    "beats": [{
      "startTick": 960,
      "voices": [{
        "index": 0,
        "duration": { "value": 8, "tuplet": { "enters": 1, "times": 1 } },
        "notes": [{ "string": 6, "fret": 5, "velocity": 95,
                    "effects": { "palmMute": true } }]
      }]
    }]
  }],
  "revision": 1, "documentId": "uuid"
}
```
Durations: `value` 1=whole ... 64=sixty-fourth, optional `dotted`/`doubleDotted`
(present only when true), `tuplet` 1:1 when normal. Voices that TuxGuitar marks
empty are omitted; a voice with no notes carries `"isRest": true`. Effect flags
appear only when set. Ties: `"tied": true` marks a continuation note.

Parameterized effects (since plugin 0.4.1):
- `"harmonic": { "type": "natural"|"artificial"|"tapped"|"pinch"|"semi",
  "data": <octave offset, artificial/tapped only> }`
- `"bend": { "points": [{ "position": 0-12, "value": <semitones> }] }` —
  position spans the note's duration; on write, an empty/missing points list
  applies a standard full-tone bend (0,0)→(6,2)→(12,2).
- Readers must also accept the legacy boolean form (`"harmonic": true` =
  natural harmonic, `"bend": true` = standard bend).

Parameterized articulations (since plugin 0.8.0) - each accepts `true`
(sensible default) or a parameter object, on read and write:
- `"tremoloPicking": { "speed": 8|16|32 }` - repick subdivision
  (default 16).
- `"trill": { "fret": <n>, "speed": 8|16|32 }` - fret 0 or absent means
  "a whole tone above the note"; speed defaults to 32.
- `"grace": { "fret": <n>, "duration": 1|2|3, "onBeat": bool,
  "transition": "none"|"slide"|"bend"|"hammer", "dead": bool }` - fret
  absent means "two frets below the note"; duration 1 = 64th, 2 = 32nd
  (default), 3 = 16th; transition defaults to hammer.
- Still a presence flag (not applied on write): tremoloBar.

Errors: `E_NO_DOCUMENT`, `E_INVALID_RANGE`.

### read_selection
Result:
```json
{
  "active": true, "trackNumber": 1, "fromMeasure": 1, "toMeasure": 2,
  "caret": { "trackNumber": 1, "measureNumber": 1, "tick": 960, "stringNumber": 6 },
  "revision": 1
}
```
`active: false` omits the range fields; the caret is reported whenever available.

### apply_changeset
Params:
```json
{
  "expectedRevision": 1,
  "changes": [{
    "type": "replaceMeasureRange",
    "trackNumber": 1,
    "fromMeasure": 1,
    "measures": [ /* Measure objects as in read_measures */ ]
  }]
}
```
Replaces the contents of `fromMeasure..` on one track with the given measures,
appending measures to the song when the range extends past its end. Protocol
v1 accepts exactly one change per change-set. Atomic: applied entirely inside
one editor lock on the UI thread, as ONE undoable edit (a single Ctrl+Z
reverts content and appended measures together). Beat `startTick`s are
interpreted relative to the measure's `startTick` (so 0-based offsets work).
Rejected with `E_STALE_REVISION` (with expected/current in the message) when
the score changed after `expectedRevision` was read.
Result: `{ newRevision, measuresReplaced, measuresAdded, notesBefore, notesAfter }`
Errors: `E_STALE_REVISION`, `E_NO_DOCUMENT`, `E_INVALID_RANGE`, `E_UNSUPPORTED`, `E_EDIT_FAILED`.

### save_copy
Opens TuxGuitar's own Save-As dialog (the user picks name/location — no
filesystem paths cross the wire). Result: `{ dialogOpened }`.

### spike_edit  *(Milestone 1 only — removed when the edit tools stabilize)*
Hard-coded undoable edit at track 1 / measure 1 / beat 1: toggles string-6
fret 5↔7, or adds a fret-5 quarter note if none exists.
Result: `{ track, measure, description, newRevision }`
Errors: `E_NO_DOCUMENT`, `E_EDIT_FAILED`, `E_LOCKED`.

### undo / redo
Result: `{ performed, newRevision }` — `performed: false` when the stack is empty.

## Methods added since (plugin 0.4+..0.7)

All follow the same conventions (auth first, camelCase, stable error codes):

- `create_track { name, strings, clef?, percussion? }` — appended track,
  black color, optional bass clef / percussion channel (bank 128)
- `change_tuning { trackNumber, strings, expectedRevision? }`
- `set_tempo { bpm, fromMeasure? }` (NOT undoable — app design)
- `set_time_signature { measure, numerator, denominator, toEnd }`
- `set_key_signature { trackNumber, measure, key, toEnd }`
- `insert_measures { at, count }` / `delete_measures { from, count }`
- `set_repeat { fromMeasure, toMeasure, repetitions }` (0 clears)
- `set_marker { measure, title }` (empty title clears)
- `play`, `play_from { measure }`, `stop`
- `toggle_action { actionId }` — WHITELISTED: metronome / count-down only
- `export_song { format }` — Save-As dialog pre-set to the format
- `render_midi` — headless MIDI to ~/.tuxguitar-mcp/render.mid
- `save_copy` — Save-As dialog

`spike_edit` remains for diagnostics.
