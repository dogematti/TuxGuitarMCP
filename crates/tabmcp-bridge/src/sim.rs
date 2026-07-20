//! A simulator of the Java bridge plugin: same protocol, canned song.
//!
//! Used by integration tests and the `tabmcp bridge-sim` subcommand so the
//! Rust side can be developed and CI-tested without a running TuxGuitar.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use serde_json::{json, Value};
use tabmcp_model::{
    error_codes, Beat, DiscoveryInfo, Duration, Header, Measure, Note, NoteEffects, Song,
    SongMetadata, StringTuning, TimeSignature, Track, Tuplet, Voice, PROTOCOL_VERSION,
    TICKS_PER_QUARTER,
};

pub struct SimHandle {
    pub port: u16,
    pub discovery_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl SimHandle {
    pub fn stop(mut self) {
        self.shutdown_now();
    }

    fn shutdown_now(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Unblock the accept loop with a throwaway connection.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.discovery_path);
    }
}

impl Drop for SimHandle {
    fn drop(&mut self) {
        if self.accept_thread.is_some() {
            self.shutdown_now();
        }
    }
}

/// The canned song the simulator serves: two tracks, four 4/4 measures at 120 bpm.
pub fn demo_song() -> Song {
    let standard = [64u8, 59, 55, 50, 45, 40];
    let bass = [43u8, 38, 33, 28];
    Song {
        metadata: SongMetadata {
            title: "Bridge Simulator Demo".into(),
            artist: "TabMCP".into(),
            ..Default::default()
        },
        tracks: vec![
            Track {
                number: 1,
                name: "Rhythm Guitar".into(),
                strings: standard
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| StringTuning {
                        number: i as u32 + 1,
                        open_pitch: p,
                    })
                    .collect(),
                program: 29,
                is_percussion: false,
                offset: 0,
                max_fret: 24,
                measure_count: 4,
            },
            Track {
                number: 2,
                name: "Bass".into(),
                strings: bass
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| StringTuning {
                        number: i as u32 + 1,
                        open_pitch: p,
                    })
                    .collect(),
                program: 33,
                is_percussion: false,
                offset: 0,
                max_fret: 24,
                measure_count: 4,
            },
        ],
        headers: (0..4u32)
            .map(|i| Header {
                number: i + 1,
                start_tick: TICKS_PER_QUARTER * (1 + 4 * i as u64),
                time_signature: TimeSignature {
                    numerator: 4,
                    denominator: 4,
                },
                tempo_bpm: 120,
                repeat_open: false,
                repeat_close: 0,
                repeat_alternative: 0,
                marker: None,
            })
            .collect(),
        revision: 0,
        document_id: "sim-doc".into(),
    }
}

/// Measures for the demo song's guitar track: measures 1-2 carry an
/// A minor pentatonic riff in eighth notes, measures 3-4 are rests.
/// (string, fret) over standard tuning: A2 C3 D3 E3 G3 A3 G3 E3 | D3 C3 A2 ...
pub fn demo_measures(from: u32, to: u32) -> Vec<Measure> {
    const RIFF: [&[(u32, u32)]; 2] = [
        &[
            (6, 5),
            (6, 8),
            (5, 5),
            (5, 7),
            (4, 5),
            (4, 7),
            (4, 5),
            (5, 7),
        ],
        &[
            (5, 5),
            (6, 8),
            (6, 5),
            (6, 8),
            (5, 5),
            (5, 7),
            (5, 5),
            (6, 5),
        ],
    ];
    let eighth = Duration {
        value: 8,
        dotted: false,
        double_dotted: false,
        tuplet: Tuplet {
            enters: 1,
            times: 1,
        },
    };
    (from..=to)
        .map(|number| {
            let measure_start = TICKS_PER_QUARTER * (1 + 4 * (number as u64 - 1));
            let beats = match RIFF.get(number as usize - 1) {
                Some(steps) => steps
                    .iter()
                    .enumerate()
                    .map(|(i, &(string, fret))| Beat {
                        start_tick: measure_start + i as u64 * (TICKS_PER_QUARTER / 2),
                        voices: vec![Voice {
                            index: 0,
                            duration: eighth.clone(),
                            is_rest: false,
                            notes: vec![Note {
                                string,
                                fret,
                                velocity: 95,
                                tied: false,
                                effects: NoteEffects::default(),
                            }],
                        }],
                    })
                    .collect(),
                None => vec![Beat {
                    start_tick: measure_start,
                    voices: vec![Voice {
                        index: 0,
                        duration: Duration {
                            value: 1,
                            dotted: false,
                            double_dotted: false,
                            tuplet: Tuplet {
                                enters: 1,
                                times: 1,
                            },
                        },
                        is_rest: true,
                        notes: Vec::new(),
                    }],
                }],
            };
            Measure {
                number,
                key_signature: 0,
                beats,
            }
        })
        .collect()
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS RNG unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Start the simulator on an ephemeral loopback port and write the discovery
/// file to `discovery_path`.
pub fn start(discovery_path: &Path) -> std::io::Result<SimHandle> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let token = random_token();

    let discovery = DiscoveryInfo {
        protocol_version: PROTOCOL_VERSION,
        port,
        token: token.clone(),
        pid: std::process::id(),
        tuxguitar_version: "simulator".into(),
        started_at_unix: unix_now(),
    };
    if let Some(parent) = discovery_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(discovery_path, serde_json::to_vec_pretty(&discovery)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(discovery_path, std::fs::Permissions::from_mode(0o600))?;
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_for_thread = Arc::clone(&shutdown);
    let accept_thread = std::thread::Builder::new()
        .name("tabmcp-sim-acceptor".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if shutdown_for_thread.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    // One client at a time, like the real plugin.
                    let _ = serve_client(stream, &token, &shutdown_for_thread);
                }
            }
        })?;

    Ok(SimHandle {
        port,
        discovery_path: discovery_path.to_path_buf(),
        shutdown,
        accept_thread: Some(accept_thread),
    })
}

struct SimState {
    authenticated: bool,
    revision: AtomicU64,
    /// Simulated fret of the spike note: None until spike_edit runs.
    spike_applied: bool,
}

fn serve_client(stream: TcpStream, token: &str, shutdown: &AtomicBool) -> std::io::Result<()> {
    // Wake up periodically so a shutdown is honored even while a client
    // is connected but idle (otherwise stop() would deadlock on join()).
    stream.set_read_timeout(Some(std::time::Duration::from_millis(200)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut state = SimState {
        authenticated: false,
        revision: AtomicU64::new(0),
        spike_applied: false,
    };

    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(()), // client disconnected
            Ok(_) => {}
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if shutdown.load(Ordering::SeqCst) {
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(e),
        }
        let response = match serde_json::from_str::<Value>(line.trim()) {
            Ok(request) => handle_request(&request, token, &mut state),
            Err(e) => error_response(
                Value::Null,
                error_codes::INTERNAL,
                &format!("bad JSON: {e}"),
            ),
        };
        let mut out = serde_json::to_string(&response)?;
        out.push('\n');
        writer.write_all(out.as_bytes())?;
    }
}

fn handle_request(request: &Value, token: &str, state: &mut SimState) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    if method != "hello" && !state.authenticated {
        return error_response(id, error_codes::NOT_AUTHENTICATED, "call hello first");
    }

    match method {
        "hello" => {
            if params.get("token").and_then(Value::as_str) != Some(token) {
                return error_response(id, error_codes::NOT_AUTHENTICATED, "invalid token");
            }
            let client_version = params.get("protocolVersion").and_then(Value::as_u64);
            if client_version != Some(PROTOCOL_VERSION as u64) {
                return error_response(
                    id,
                    error_codes::PROTOCOL_VERSION,
                    &format!("simulator speaks protocol {PROTOCOL_VERSION}"),
                );
            }
            state.authenticated = true;
            result_response(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "serverInfo": {"tuxguitarVersion": "simulator", "pluginVersion": env!("CARGO_PKG_VERSION")},
                    "capabilities": ["read", "edit", "undo"],
                }),
            )
        }
        "ping" => result_response(
            id,
            json!({
                "revision": state.revision.load(Ordering::SeqCst),
                "documentOpen": true,
                "playing": false,
            }),
        ),
        "read_song" => {
            let mut song = demo_song();
            song.revision = state.revision.load(Ordering::SeqCst);
            result_response(
                id,
                serde_json::to_value(song).expect("demo song serializes"),
            )
        }
        "read_measures" => {
            let track = params
                .get("trackNumber")
                .and_then(Value::as_u64)
                .unwrap_or(1) as u32;
            let from = params
                .get("fromMeasure")
                .and_then(Value::as_u64)
                .unwrap_or(1) as u32;
            let to = params
                .get("toMeasure")
                .and_then(Value::as_u64)
                .unwrap_or(4)
                .min(4) as u32;
            if track > 2 || from == 0 || from > to {
                return error_response(id, error_codes::INVALID_RANGE, "bad track/measure range");
            }
            let measures = if track == 1 {
                demo_measures(from, to)
            } else {
                demo_measures(3, 3) // bass track: rests only in the sim
            };
            result_response(
                id,
                json!({
                    "trackNumber": track,
                    "fromMeasure": from,
                    "toMeasure": to,
                    "measures": measures,
                    "revision": state.revision.load(Ordering::SeqCst),
                    "documentId": "sim-doc",
                }),
            )
        }
        "read_selection" => result_response(
            id,
            json!({
                "active": true,
                "trackNumber": 1,
                "fromMeasure": 1,
                "toMeasure": 2,
                "caret": {
                    "trackNumber": 1,
                    "measureNumber": 1,
                    "tick": TICKS_PER_QUARTER,
                    "stringNumber": 6,
                },
                "revision": state.revision.load(Ordering::SeqCst),
            }),
        ),
        "spike_edit" => {
            state.spike_applied = true;
            let new_revision = state.revision.fetch_add(1, Ordering::SeqCst) + 1;
            result_response(
                id,
                json!({
                    "track": 1,
                    "measure": 1,
                    "description": "simulated: added E5 (string 6, fret 0) at measure 1 beat 1",
                    "newRevision": new_revision,
                }),
            )
        }
        "undo" | "redo" => {
            let performed = if method == "undo" {
                state.spike_applied
            } else {
                !state.spike_applied
            };
            if performed {
                state.spike_applied = method != "undo";
                state.revision.fetch_add(1, Ordering::SeqCst);
            }
            result_response(
                id,
                json!({
                    "performed": performed,
                    "newRevision": state.revision.load(Ordering::SeqCst),
                }),
            )
        }
        _ => error_response(
            id,
            error_codes::UNSUPPORTED,
            &format!("unknown method: {method}"),
        ),
    }
}

fn result_response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error_response(id: Value, code: &str, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32000, "message": message, "data": {"code": code}},
    })
}
