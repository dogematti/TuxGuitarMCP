//! Wire types shared between the Rust side and the TuxGuitar Java bridge.
//!
//! Every type here crosses the JSON-RPC socket, so all fields serialize as
//! camelCase and unknown fields are tolerated (no `deny_unknown_fields`) to
//! keep the protocol forward-compatible. `docs/PROTOCOL.md` is the normative
//! description; these structs are the reference implementation.

use serde::{Deserialize, Serialize};

/// Bridge protocol version implemented by this crate.
pub const PROTOCOL_VERSION: u32 = 1;

/// Ticks per quarter note, matching TuxGuitar's `TGDuration.QUARTER_TIME`.
pub const TICKS_PER_QUARTER: u64 = 960;

/// Contents of `~/.tuxguitar-mcp/bridge.json`, written by the Java plugin
/// while its socket server is listening.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryInfo {
    pub protocol_version: u32,
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub tuxguitar_version: String,
    pub started_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloParams {
    pub token: String,
    pub protocol_version: u32,
    pub client_info: ClientInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub tuxguitar_version: String,
    pub plugin_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloResult {
    pub protocol_version: u32,
    pub server_info: ServerInfo,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub revision: u64,
    pub document_open: bool,
    pub playing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeSignature {
    pub numerator: u32,
    /// Denominator as the note value (4 = quarter, 8 = eighth, ...).
    pub denominator: u32,
}

/// One measure header: song-wide per-measure structure (shared by all tracks).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    /// 1-based measure number.
    pub number: u32,
    pub start_tick: u64,
    pub time_signature: TimeSignature,
    pub tempo_bpm: u32,
    pub repeat_open: bool,
    /// 0 = no repeat close; otherwise the number of repetitions.
    pub repeat_close: u32,
    pub repeat_alternative: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

/// One string of a track. `number` is 1-based, 1 = highest-sounding string
/// in TuxGuitar's convention. `open_pitch` is the MIDI pitch of the open string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StringTuning {
    pub number: u32,
    pub open_pitch: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// 1-based track number as shown in TuxGuitar.
    pub number: u32,
    pub name: String,
    pub strings: Vec<StringTuning>,
    /// MIDI program of the track's channel.
    pub program: u16,
    pub is_percussion: bool,
    /// Transposition offset in semitones (TuxGuitar `TGTrack.offset`).
    pub offset: i32,
    pub max_fret: u32,
    pub measure_count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub author: String,
    pub comments: String,
}

/// Result of `read_song`: everything about the open document except note data
/// (notes come from `read_measures`, added in Phase 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub metadata: SongMetadata,
    pub tracks: Vec<Track>,
    pub headers: Vec<Header>,
    /// Monotonic edit revision maintained by the bridge; all writes are
    /// validated against it.
    pub revision: u64,
    /// Rotates when a different document is loaded, so revisions from one
    /// song can never validate against another.
    pub document_id: String,
}

/// Result of the Milestone-1 `spike_edit` method (hard-coded undoable edit).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeEditResult {
    pub track: u32,
    pub measure: u32,
    pub description: String,
    pub new_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub performed: bool,
    pub new_revision: u64,
}

/// Stable application-level error codes carried in JSON-RPC `error.data.code`.
pub mod error_codes {
    pub const NOT_AUTHENTICATED: &str = "E_NOT_AUTHENTICATED";
    pub const PROTOCOL_VERSION: &str = "E_PROTOCOL_VERSION";
    pub const NO_DOCUMENT: &str = "E_NO_DOCUMENT";
    pub const STALE_REVISION: &str = "E_STALE_REVISION";
    pub const INVALID_RANGE: &str = "E_INVALID_RANGE";
    pub const UNSUPPORTED: &str = "E_UNSUPPORTED";
    pub const EDIT_FAILED: &str = "E_EDIT_FAILED";
    pub const LOCKED: &str = "E_LOCKED";
    pub const INTERNAL: &str = "E_INTERNAL";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_round_trips_through_json() {
        let song = Song {
            metadata: SongMetadata {
                title: "Test".into(),
                ..Default::default()
            },
            tracks: vec![Track {
                number: 1,
                name: "Guitar".into(),
                strings: vec![
                    StringTuning {
                        number: 1,
                        open_pitch: 64,
                    },
                    StringTuning {
                        number: 6,
                        open_pitch: 40,
                    },
                ],
                program: 29,
                is_percussion: false,
                offset: 0,
                max_fret: 24,
                measure_count: 4,
            }],
            headers: vec![Header {
                number: 1,
                start_tick: TICKS_PER_QUARTER,
                time_signature: TimeSignature {
                    numerator: 4,
                    denominator: 4,
                },
                tempo_bpm: 120,
                repeat_open: false,
                repeat_close: 0,
                repeat_alternative: 0,
                marker: None,
            }],
            revision: 7,
            document_id: "doc-1".into(),
        };
        let json = serde_json::to_string(&song).unwrap();
        assert!(
            json.contains("\"openPitch\":64"),
            "wire format must be camelCase: {json}"
        );
        assert!(json.contains("\"startTick\":960"));
        let back: Song = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tracks[0].strings, song.tracks[0].strings);
        assert_eq!(back.revision, 7);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let json = r#"{"revision":1,"documentOpen":true,"playing":false,"futureField":123}"#;
        let ping: PingResult = serde_json::from_str(json).unwrap();
        assert!(ping.document_open);
    }
}
