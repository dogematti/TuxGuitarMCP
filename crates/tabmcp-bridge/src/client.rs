//! Synchronous JSON-RPC client for the bridge socket.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tabmcp_model::{
    ApplyResult, ClientInfo, DiscoveryInfo, HelloParams, HelloResult, Measure, MeasureRange,
    PingResult, SaveCopyResult, Selection, Song, SpikeEditResult, StringTuning, UndoResult,
    PROTOCOL_VERSION,
};

use crate::discovery::read_discovery;
use crate::error::BridgeError;

const READ_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

pub struct BridgeClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
    hello: HelloResult,
    discovery: DiscoveryInfo,
}

impl BridgeClient {
    /// Read the discovery file, connect, and authenticate.
    pub fn connect(discovery_path: &Path) -> Result<Self, BridgeError> {
        let discovery = read_discovery(discovery_path)?;
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], discovery.port));
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|source| {
            BridgeError::Unreachable {
                port: discovery.port,
                source,
            }
        })?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_nodelay(true)?;

        let mut client = BridgeClient {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 0,
            hello: HelloResult {
                protocol_version: 0,
                server_info: tabmcp_model::ServerInfo {
                    tuxguitar_version: String::new(),
                    plugin_version: String::new(),
                },
                capabilities: Vec::new(),
            },
            discovery,
        };

        let hello: HelloResult = client.call(
            "hello",
            &HelloParams {
                token: client.discovery.token.clone(),
                protocol_version: PROTOCOL_VERSION,
                client_info: ClientInfo {
                    name: "tabmcp".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                },
            },
        )?;
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(BridgeError::VersionMismatch {
                ours: PROTOCOL_VERSION,
                theirs: hello.protocol_version,
            });
        }
        client.hello = hello;
        Ok(client)
    }

    pub fn hello_info(&self) -> &HelloResult {
        &self.hello
    }

    pub fn discovery_info(&self) -> &DiscoveryInfo {
        &self.discovery
    }

    pub fn ping(&mut self) -> Result<PingResult, BridgeError> {
        self.call("ping", &json!({}))
    }

    pub fn read_song(&mut self) -> Result<Song, BridgeError> {
        self.call("read_song", &json!({}))
    }

    pub fn read_measures(
        &mut self,
        track_number: u32,
        from_measure: u32,
        to_measure: u32,
    ) -> Result<MeasureRange, BridgeError> {
        self.call(
            "read_measures",
            &json!({
                "trackNumber": track_number,
                "fromMeasure": from_measure,
                "toMeasure": to_measure,
            }),
        )
    }

    pub fn read_selection(&mut self) -> Result<Selection, BridgeError> {
        self.call("read_selection", &json!({}))
    }

    /// Replace the contents of `from_measure..` on a track with `measures`,
    /// atomically and undoably, iff the score is still at `expected_revision`.
    pub fn apply_replace_measures(
        &mut self,
        track_number: u32,
        from_measure: u32,
        measures: &[Measure],
        expected_revision: u64,
    ) -> Result<ApplyResult, BridgeError> {
        self.call(
            "apply_changeset",
            &json!({
                "expectedRevision": expected_revision,
                "changes": [{
                    "type": "replaceMeasureRange",
                    "trackNumber": track_number,
                    "fromMeasure": from_measure,
                    "measures": measures,
                }],
            }),
        )
    }

    /// Open TuxGuitar's Save-As dialog so the user can save a copy.
    pub fn save_copy(&mut self) -> Result<SaveCopyResult, BridgeError> {
        self.call("save_copy", &json!({}))
    }

    /// Create a new track; `strings` are open pitches, string 1 first.
    /// `clef` is "treble" (default) or "bass"; `percussion` marks the
    /// track's channel as a drum channel.
    pub fn create_track(
        &mut self,
        name: &str,
        strings: &[StringTuning],
        clef: Option<&str>,
        percussion: bool,
    ) -> Result<serde_json::Value, BridgeError> {
        let mut params = json!({ "name": name, "strings": strings });
        if let Some(clef) = clef {
            params["clef"] = json!(clef); // omitted entirely when unset
        }
        if percussion {
            params["percussion"] = json!(true);
        }
        self.call("create_track", &params)
    }

    pub fn change_tuning(
        &mut self,
        track_number: u32,
        strings: &[StringTuning],
        expected_revision: u64,
    ) -> Result<serde_json::Value, BridgeError> {
        self.call(
            "change_tuning",
            &json!({
                "trackNumber": track_number,
                "strings": strings,
                "expectedRevision": expected_revision,
            }),
        )
    }

    /// Set repeat signs: open at `from_measure`, close at `to_measure` with
    /// the given repeat count (0 clears the repeat).
    pub fn set_repeat(
        &mut self,
        from_measure: u32,
        to_measure: u32,
        repetitions: u32,
    ) -> Result<serde_json::Value, BridgeError> {
        self.call(
            "set_repeat",
            &json!({
                "fromMeasure": from_measure,
                "toMeasure": to_measure,
                "repetitions": repetitions,
            }),
        )
    }

    /// Change the tempo: whole song when `from_measure` is None, otherwise
    /// from that measure to the end.
    pub fn set_tempo(
        &mut self,
        bpm: u32,
        from_measure: Option<u32>,
    ) -> Result<serde_json::Value, BridgeError> {
        let mut params = json!({ "bpm": bpm });
        if let Some(measure) = from_measure {
            params["fromMeasure"] = json!(measure);
        }
        self.call("set_tempo", &params)
    }

    /// Open TuxGuitar's export dialog pre-set to a format ("mid", "MIDI", ...).
    pub fn export_song(&mut self, format: &str) -> Result<serde_json::Value, BridgeError> {
        self.call("export_song", &json!({ "format": format }))
    }

    /// Headless MIDI render to the bridge's fixed scratch path.
    pub fn render_midi(&mut self) -> Result<serde_json::Value, BridgeError> {
        self.call("render_midi", &json!({}))
    }

    pub fn play(&mut self) -> Result<serde_json::Value, BridgeError> {
        self.call("play", &json!({}))
    }

    /// Move the caret/playback position to a measure and start playing.
    pub fn play_from(&mut self, measure: u32) -> Result<serde_json::Value, BridgeError> {
        self.call("play_from", &json!({ "measure": measure }))
    }

    pub fn stop(&mut self) -> Result<serde_json::Value, BridgeError> {
        self.call("stop", &json!({}))
    }

    pub fn spike_edit(&mut self) -> Result<SpikeEditResult, BridgeError> {
        self.call("spike_edit", &json!({}))
    }

    pub fn undo(&mut self) -> Result<UndoResult, BridgeError> {
        self.call("undo", &json!({}))
    }

    pub fn redo(&mut self) -> Result<UndoResult, BridgeError> {
        self.call("redo", &json!({}))
    }

    /// One JSON-RPC round trip.
    fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: &P,
    ) -> Result<R, BridgeError> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;

        let mut response_line = String::new();
        let read = self.reader.read_line(&mut response_line)?;
        if read == 0 {
            return Err(BridgeError::Malformed("connection closed by bridge".into()));
        }
        let response: Value = serde_json::from_str(response_line.trim())?;

        if response.get("id").and_then(Value::as_u64) != Some(id) {
            return Err(BridgeError::Malformed(format!(
                "response id does not match request id {id}: {response}"
            )));
        }
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string();
            let code = error
                .get("data")
                .and_then(|d| d.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("E_INTERNAL")
                .to_string();
            return Err(BridgeError::Rejected { code, message });
        }
        let result = response.get("result").ok_or_else(|| {
            BridgeError::Malformed(format!("response without result: {response}"))
        })?;
        Ok(serde_json::from_value(result.clone())?)
    }
}
