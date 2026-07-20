//! The MCP server: exposes the TuxGuitar bridge and the theory engine as
//! MCP tools over stdio.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorData, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tabmcp_bridge::{BridgeClient, BridgeError};
use tabmcp_model::{MeasureRange, Selection, Song, TICKS_PER_QUARTER};
use tabmcp_theory::{detect_scales, explain, note_name, NoteEvent};

/// Read this many measures per `get_measures` call at most: keeps single
/// responses focused instead of dumping whole songs into context.
const MAX_MEASURES_PER_READ: u32 = 32;

pub fn run(discovery_path: &Path) -> anyhow_free::Result {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start async runtime: {e}"))?;
    runtime.block_on(async {
        eprintln!("[tabmcp] MCP server starting on stdio");
        let service = TabMcp::new(discovery_path.to_path_buf())
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| format!("MCP initialization failed: {e}"))?;
        service
            .waiting()
            .await
            .map_err(|e| format!("MCP server terminated abnormally: {e}"))?;
        Ok(())
    })
}

/// Tiny local Result alias to avoid pulling in anyhow for one function.
mod anyhow_free {
    pub type Result = std::result::Result<(), String>;
}

#[derive(Clone)]
pub struct TabMcp {
    bridge: Arc<Mutex<Option<BridgeClient>>>,
    discovery_path: PathBuf,
    tool_router: ToolRouter<Self>,
}

// ---------- tool parameter and result types ----------

#[derive(Deserialize, JsonSchema)]
struct MeasureRangeParams {
    /// Track number as shown in TuxGuitar (1-based).
    track_number: u32,
    /// First measure to read (1-based, inclusive).
    from_measure: u32,
    /// Last measure to read (inclusive). At most 32 measures per call.
    to_measure: u32,
}

#[derive(Default, Deserialize, JsonSchema)]
struct AnalysisScopeParams {
    /// Track number (1-based). Omit to use the active selection's track,
    /// falling back to track 1.
    #[serde(default)]
    track_number: Option<u32>,
    /// First measure of the passage (1-based). Omit to use the selection,
    /// falling back to the whole track.
    #[serde(default)]
    from_measure: Option<u32>,
    /// Last measure of the passage (inclusive).
    #[serde(default)]
    to_measure: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BridgeStatus {
    connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tuxguitar_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capabilities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TrackSummary {
    number: u32,
    name: String,
    /// Open-string notes, high to low (e.g. ["E4","B3","G3","D3","A2","E2"]).
    tuning: Vec<String>,
    program: u16,
    is_percussion: bool,
    measure_count: u32,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ScoreSummary {
    title: String,
    artist: String,
    tracks: Vec<TrackSummary>,
    measure_count: u32,
    /// Time signature of the first measure, e.g. "4/4".
    time_signature: String,
    /// Tempo of the first measure in beats per minute.
    tempo_bpm: u32,
    revision: u64,
    document_id: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ScaleCandidateOut {
    root: String,
    scale: String,
    /// 0..1; how well the passage fits this scale.
    confidence: f64,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct KeyScaleResult {
    /// The passage that was analyzed, e.g. "track 1, measures 1-2".
    scope: String,
    note_count: usize,
    candidates: Vec<ScaleCandidateOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tonal_center: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UndoRedoResult {
    performed: bool,
    revision: u64,
}

#[derive(Deserialize, JsonSchema)]
struct ReplaceMeasuresParams {
    /// Track number as shown in TuxGuitar (1-based).
    track_number: u32,
    /// First measure to replace (1-based). The given measures land at
    /// from_measure, from_measure+1, ...; measures past the end of the song
    /// are appended automatically.
    from_measure: u32,
    /// The new measure contents. Beat startTicks may be measure-relative
    /// (startTick 0 = start of the measure) or absolute ticks.
    measures: Vec<tabmcp_model::Measure>,
    /// False (default): return a preview and the revision to confirm with.
    /// True: apply the edit — requires expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call. The edit is rejected if
    /// the score changed since.
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct TransposeParams {
    /// Semitones to transpose by (positive = up, negative = down).
    semitones: i32,
    /// Track number (1-based). Omit to use the active selection's track.
    #[serde(default)]
    track_number: Option<u32>,
    /// First measure. Omit to use the selection, falling back to the whole track.
    #[serde(default)]
    from_measure: Option<u32>,
    /// Last measure (inclusive).
    #[serde(default)]
    to_measure: Option<u32>,
    /// False (default): preview only. True: apply — requires expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct EditOutcome {
    /// False = this was a dry-run preview; nothing was changed.
    applied: bool,
    /// Human-readable description of what happened / would happen.
    summary: String,
    /// Score revision: pass as expected_revision when confirming a preview.
    revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    measures_added: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes_before: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes_after: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct OptimizeFingeringParams {
    /// Track number (1-based). Omit to use the active selection's track.
    #[serde(default)]
    track_number: Option<u32>,
    /// First measure. Omit to use the selection, falling back to the whole track.
    #[serde(default)]
    from_measure: Option<u32>,
    /// Last measure (inclusive).
    #[serde(default)]
    to_measure: Option<u32>,
    /// False (default): preview only. True: apply — requires expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
    /// Lowest fret allowed for fretted notes (open strings stay allowed).
    #[serde(default)]
    min_fret: Option<u32>,
    /// Highest fret allowed for fretted notes (e.g. 12 to stay low on the neck).
    #[serde(default)]
    max_fret_limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct GenerateParams {
    /// Source track to derive from (1-based). Omit to use the selection's
    /// track, falling back to track 1.
    #[serde(default)]
    source_track: Option<u32>,
    /// First measure of the source passage. Omit to use the selection,
    /// falling back to the whole track.
    #[serde(default)]
    from_measure: Option<u32>,
    /// Last measure (inclusive).
    #[serde(default)]
    to_measure: Option<u32>,
    /// Harmony only: "third" (default) or "sixth".
    #[serde(default)]
    interval: Option<String>,
    /// False (default): preview what would be generated. True: create the
    /// new track and write the line — requires expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct SetTempoParams {
    /// New tempo in beats per minute (1..320).
    bpm: u32,
    /// Apply from this measure to the end. Omit to change the whole song.
    #[serde(default)]
    from_measure: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct ExportParams {
    /// Export format by name or extension: "mid"/"MIDI" for multitrack MIDI
    /// (default). Other formats depend on installed TuxGuitar exporters
    /// (e.g. Guitar Pro); unknown formats return the available list.
    #[serde(default)]
    format: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct SetRepeatParams {
    /// Measure where the repeat opens (1-based).
    from_measure: u32,
    /// Measure where the repeat closes (inclusive).
    to_measure: u32,
    /// How many times the range repeats (default 2). 0 clears the repeat.
    #[serde(default)]
    repetitions: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct CreateTrackParams {
    /// Track name shown in TuxGuitar (e.g. "7-String Rhythm").
    name: String,
    /// Open-string note names, HIGHEST string first (e.g. 7-string A standard
    /// = ["D4","A3","F3","C3","G2","D2","A1"]). Provide either this or preset.
    #[serde(default)]
    tuning: Option<Vec<String>>,
    /// A preset name instead of explicit tuning. One of: "6-string standard",
    /// "6-string drop D", "6-string E-flat", "6-string drop C",
    /// "7-string B standard", "7-string A standard", "8-string F# standard",
    /// "4-string bass", "5-string bass".
    #[serde(default)]
    preset: Option<String>,
    /// Notation clef: "treble" (default) or "bass".
    #[serde(default)]
    clef: Option<String>,
    /// True to make this a percussion (drum) track — note frets become
    /// General-MIDI drum keys (36 kick, 38 snare, 42 closed hi-hat, ...).
    #[serde(default)]
    percussion: bool,
}

#[derive(Deserialize, JsonSchema)]
struct ChangeTuningParams {
    /// Track number (1-based).
    track_number: u32,
    /// Open-string note names, highest string first. Either this or preset.
    #[serde(default)]
    tuning: Option<Vec<String>>,
    /// A preset name (same options as in tuxguitar_create_track).
    #[serde(default)]
    preset: Option<String>,
    /// False (default): preview only. True: apply — requires expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
}

/// Resolve preset/tuning params to string tunings (string 1 = highest first).
fn resolve_tuning(
    tuning: &Option<Vec<String>>,
    preset: &Option<String>,
) -> Result<Vec<tabmcp_model::StringTuning>, ErrorData> {
    let pitches: Vec<u8> = match (tuning, preset) {
        (Some(names), _) if !names.is_empty() => names
            .iter()
            .map(|name| {
                tabmcp_theory::parse_note(name).ok_or_else(|| {
                    ErrorData::invalid_params(
                        format!("cannot parse note name '{name}' (expected e.g. \"A1\", \"F#3\")"),
                        None,
                    )
                })
            })
            .collect::<Result<_, _>>()?,
        (_, Some(preset_name)) => tabmcp_theory::tuning_preset(preset_name)
            .ok_or_else(|| {
                let known: Vec<&str> = tabmcp_theory::TUNING_PRESETS
                    .iter()
                    .map(|(name, _)| *name)
                    .collect();
                ErrorData::invalid_params(
                    format!(
                        "unknown preset '{preset_name}'; known presets: {}",
                        known.join(", ")
                    ),
                    None,
                )
            })?
            .to_vec(),
        _ => {
            return Err(ErrorData::invalid_params(
                "provide either tuning (note names, highest string first) or preset",
                None,
            ))
        }
    };
    Ok(pitches
        .iter()
        .enumerate()
        .map(|(i, &open_pitch)| tabmcp_model::StringTuning {
            number: i as u32 + 1,
            open_pitch,
        })
        .collect())
}

fn tuning_names(strings: &[tabmcp_model::StringTuning]) -> String {
    strings
        .iter()
        .map(|s| note_name(s.open_pitch))
        .collect::<Vec<_>>()
        .join(" ")
}

fn count_notes(measures: &[tabmcp_model::Measure]) -> u32 {
    measures
        .iter()
        .flat_map(|m| &m.beats)
        .flat_map(|b| &b.voices)
        .map(|v| v.notes.len() as u32)
        .sum()
}

// ---------- tools ----------

#[tool_router]
impl TabMcp {
    pub fn new(discovery_path: PathBuf) -> Self {
        Self {
            bridge: Arc::new(Mutex::new(None)),
            discovery_path,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Check whether TuxGuitar is running with the TabMCP bridge plugin, and report versions, capabilities, and the current score revision. Call this first if other tools fail.",
        annotations(title = "Bridge status", read_only_hint = true)
    )]
    async fn tuxguitar_get_bridge_status(&self) -> Json<BridgeStatus> {
        let result = self
            .call_bridge(|client| {
                let hello = client.hello_info().clone();
                let ping = client.ping()?;
                Ok((hello, ping))
            })
            .await;
        Json(match result {
            Ok((hello, ping)) => BridgeStatus {
                connected: true,
                tuxguitar_version: Some(hello.server_info.tuxguitar_version),
                plugin_version: Some(hello.server_info.plugin_version),
                protocol_version: Some(hello.protocol_version),
                capabilities: Some(hello.capabilities),
                document_open: Some(ping.document_open),
                revision: Some(ping.revision),
                error: None,
            },
            Err(error) => BridgeStatus {
                connected: false,
                tuxguitar_version: None,
                plugin_version: None,
                protocol_version: None,
                capabilities: None,
                document_open: None,
                revision: None,
                error: Some(error.message),
            },
        })
    }

    #[tool(
        description = "Summary of the score currently open in TuxGuitar: title, tracks with tunings, measure count, time signature, tempo, and the revision id used for edits.",
        annotations(title = "Score summary", read_only_hint = true)
    )]
    async fn tuxguitar_get_score_summary(&self) -> Result<Json<ScoreSummary>, ErrorData> {
        let song = self.fetch_song().await?;
        let first = song.headers.first();
        Ok(Json(ScoreSummary {
            title: song.metadata.title.clone(),
            artist: song.metadata.artist.clone(),
            tracks: song
                .tracks
                .iter()
                .map(|track| TrackSummary {
                    number: track.number,
                    name: track.name.clone(),
                    tuning: track
                        .strings
                        .iter()
                        .map(|s| note_name(s.open_pitch))
                        .collect(),
                    program: track.program,
                    is_percussion: track.is_percussion,
                    measure_count: track.measure_count,
                })
                .collect(),
            measure_count: song.headers.len() as u32,
            time_signature: first
                .map(|h| {
                    format!(
                        "{}/{}",
                        h.time_signature.numerator, h.time_signature.denominator
                    )
                })
                .unwrap_or_else(|| "?".into()),
            tempo_bpm: first.map(|h| h.tempo_bpm).unwrap_or(0),
            revision: song.revision,
            document_id: song.document_id.clone(),
        }))
    }

    #[tool(
        description = "Read the note content of a measure range on one track: beats, voices, durations, string/fret positions, and effect flags (palm mute, bends, slides, ...). Reads at most 32 measures per call.",
        annotations(title = "Read measures", read_only_hint = true)
    )]
    async fn tuxguitar_get_measures(
        &self,
        params: Parameters<MeasureRangeParams>,
    ) -> Result<Json<MeasureRange>, ErrorData> {
        let Parameters(p) = params;
        if p.from_measure == 0 || p.to_measure < p.from_measure {
            return Err(ErrorData::invalid_params(
                "from_measure must be >= 1 and <= to_measure",
                None,
            ));
        }
        if p.to_measure - p.from_measure + 1 > MAX_MEASURES_PER_READ {
            return Err(ErrorData::invalid_params(
                format!(
                    "range too large: read at most {MAX_MEASURES_PER_READ} measures per call \
                     (requested {})",
                    p.to_measure - p.from_measure + 1
                ),
                None,
            ));
        }
        self.call_bridge(move |client| {
            client.read_measures(p.track_number, p.from_measure, p.to_measure)
        })
        .await
        .map(Json)
        .map_err(BridgeCallError::into_error_data)
    }

    #[tool(
        description = "Read the current selection and caret position in TuxGuitar (which track and measure range the user has highlighted).",
        annotations(title = "Read selection", read_only_hint = true)
    )]
    async fn tuxguitar_get_selection(&self) -> Result<Json<Selection>, ErrorData> {
        self.call_bridge(|client| client.read_selection())
            .await
            .map(Json)
            .map_err(BridgeCallError::into_error_data)
    }

    #[tool(
        description = "Detect the most likely key/scale and tonal center of a passage. Defaults to the user's active selection; pass track/measure range to analyze something else.",
        annotations(title = "Detect key & scale", read_only_hint = true)
    )]
    async fn tuxguitar_detect_key_and_scale(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<Json<KeyScaleResult>, ErrorData> {
        let (scope, events) = self.collect_events(params.0).await?;
        let candidates = detect_scales(&events);
        Ok(Json(KeyScaleResult {
            scope,
            note_count: events.len(),
            tonal_center: candidates.first().map(|c| c.root.clone()),
            candidates: candidates
                .into_iter()
                .map(|c| ScaleCandidateOut {
                    root: c.root,
                    scale: c.scale,
                    confidence: (c.confidence * 100.0).round() / 100.0,
                })
                .collect(),
        }))
    }

    #[tool(
        description = "Explain a passage in plain language: its notes, range, melodic intervals, likely scale and tonal center. Defaults to the user's active selection.",
        annotations(title = "Explain selection", read_only_hint = true)
    )]
    async fn tuxguitar_explain_selection(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<String, ErrorData> {
        let (scope, events) = self.collect_events(params.0).await?;
        Ok(format!("Analyzed {scope}.\n\n{}", explain(&events)))
    }

    #[tool(
        description = "Undo the most recent edit in TuxGuitar (equivalent to Ctrl+Z / Cmd+Z). Returns whether anything was undone.",
        annotations(title = "Undo", read_only_hint = false, destructive_hint = false)
    )]
    async fn tuxguitar_undo(&self) -> Result<Json<UndoRedoResult>, ErrorData> {
        self.call_bridge(|client| client.undo())
            .await
            .map(|r| {
                Json(UndoRedoResult {
                    performed: r.performed,
                    revision: r.new_revision,
                })
            })
            .map_err(BridgeCallError::into_error_data)
    }

    #[tool(
        description = "Redo the most recently undone edit in TuxGuitar. Returns whether anything was redone.",
        annotations(title = "Redo", read_only_hint = false, destructive_hint = false)
    )]
    async fn tuxguitar_redo(&self) -> Result<Json<UndoRedoResult>, ErrorData> {
        self.call_bridge(|client| client.redo())
            .await
            .map(|r| {
                Json(UndoRedoResult {
                    performed: r.performed,
                    revision: r.new_revision,
                })
            })
            .map_err(BridgeCallError::into_error_data)
    }

    #[tool(
        description = "Write tablature into the open score: replace a measure range on one track with new measures (notes as string+fret; measures past the end of the song are appended automatically). Effects per note: booleans (palmMute, vibrato, slide, hammer, letRing, staccato, deadNote, ...) plus parameterized harmonic {type: natural|artificial|tapped|pinch|semi} and bend {points: [{position 0-12, value in semitones}]} (empty points = standard full-tone bend). TWO-STEP SAFETY: call without confirm to get a preview and revision, then call again with confirm=true and expected_revision to apply. The edit is atomic and undoable with Cmd+Z.",
        annotations(
            title = "Replace measures",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn tuxguitar_replace_measures(
        &self,
        params: Parameters<ReplaceMeasuresParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(mut p) = params;
        if p.measures.is_empty() || p.measures.len() > MAX_MEASURES_PER_READ as usize {
            return Err(ErrorData::invalid_params(
                format!("provide 1..={MAX_MEASURES_PER_READ} measures"),
                None,
            ));
        }
        if p.from_measure == 0 {
            return Err(ErrorData::invalid_params("from_measure must be >= 1", None));
        }
        // Renumber sequentially from from_measure so callers can't create gaps.
        for (i, measure) in p.measures.iter_mut().enumerate() {
            measure.number = p.from_measure + i as u32;
        }
        let to_measure = p.from_measure + p.measures.len() as u32 - 1;
        let notes_after = count_notes(&p.measures);

        let song = self.fetch_song().await?;
        let song_len = song.headers.len() as u32;
        if !song.tracks.iter().any(|t| t.number == p.track_number) {
            return Err(ErrorData::invalid_params(
                format!("track {} does not exist", p.track_number),
                None,
            ));
        }
        let notes_before = if p.from_measure <= song_len {
            let track_number = p.track_number;
            let from = p.from_measure;
            let to = to_measure.min(song_len);
            let existing = self
                .call_bridge(move |client| client.read_measures(track_number, from, to))
                .await
                .map_err(BridgeCallError::into_error_data)?;
            count_notes(&existing.measures)
        } else {
            0
        };
        let measures_added = to_measure.saturating_sub(song_len);

        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would replace measures {}-{} on track {} \
                     ({} notes now, {} notes after{}). To apply, call again with confirm=true \
                     and expected_revision={}.",
                    p.from_measure,
                    to_measure,
                    p.track_number,
                    notes_before,
                    notes_after,
                    if measures_added > 0 {
                        format!(", appending {measures_added} new measure(s) to the song")
                    } else {
                        String::new()
                    },
                    song.revision,
                ),
                revision: song.revision,
                measures_added: Some(measures_added),
                notes_before: Some(notes_before),
                notes_after: Some(notes_after),
            }));
        }

        let expected_revision = p.expected_revision.ok_or_else(|| {
            ErrorData::invalid_params(
                "confirm=true requires expected_revision (from the preview call)",
                None,
            )
        })?;
        let result = self
            .call_bridge(move |client| {
                client.apply_replace_measures(
                    p.track_number,
                    p.from_measure,
                    &p.measures,
                    expected_revision,
                )
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied: replaced {} measure(s){} on track {}; {} notes -> {} notes. \
                 The user can undo with Cmd+Z.",
                result.measures_replaced,
                if result.measures_added > 0 {
                    format!(" (added {} new)", result.measures_added)
                } else {
                    String::new()
                },
                p.track_number,
                result.notes_before,
                result.notes_after,
            ),
            revision: result.new_revision,
            measures_added: Some(result.measures_added),
            notes_before: Some(result.notes_before),
            notes_after: Some(result.notes_after),
        }))
    }

    #[tool(
        description = "Transpose a passage by N semitones, re-fretting on the same strings. Defaults to the user's active selection. TWO-STEP SAFETY: preview first, then confirm=true with expected_revision. Fails with a per-note list if any note would fall off the fretboard.",
        annotations(title = "Transpose", read_only_hint = false, destructive_hint = true)
    )]
    async fn tuxguitar_transpose(
        &self,
        params: Parameters<TransposeParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        let (song, selection) = self
            .call_bridge(|client| {
                let song = client.read_song()?;
                let selection = client.read_selection()?;
                Ok((song, selection))
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;

        let track_number = p
            .track_number
            .or(if selection.active {
                selection.track_number
            } else {
                None
            })
            .unwrap_or(1);
        let song_len = song.headers.len() as u32;
        let (from, to) = match (p.from_measure, p.to_measure) {
            (Some(from), Some(to)) => (from, to),
            (Some(from), None) => (from, from),
            (None, _) if selection.active => (
                selection.from_measure.unwrap_or(1),
                selection.to_measure.unwrap_or(song_len),
            ),
            _ => (1, song_len.max(1)),
        };
        if from == 0 || to < from || to > song_len {
            return Err(ErrorData::invalid_params(
                format!("invalid measure range {from}-{to}: the score has measures 1-{song_len}"),
                None,
            ));
        }
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track_number} does not exist"), None)
            })?;
        let max_fret = if track.max_fret > 0 {
            track.max_fret
        } else {
            24
        };

        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let mut measures = range.measures;
        let note_count = count_notes(&measures);
        let problems = tabmcp_theory::transpose_measures(&mut measures, p.semitones, max_fret);
        if !problems.is_empty() {
            let listing: Vec<String> = problems
                .iter()
                .take(10)
                .map(|problem| {
                    format!(
                        "measure {}: string {} fret {} -> {}",
                        problem.measure, problem.string, problem.old_fret, problem.target_fret
                    )
                })
                .collect();
            return Err(ErrorData::invalid_params(
                format!(
                    "cannot transpose by {} semitones on the same strings — {} note(s) would \
                     leave the fretboard (0..{}): {}. Try a smaller interval or the octave in \
                     the other direction.",
                    p.semitones,
                    problems.len(),
                    max_fret,
                    listing.join("; "),
                ),
                None,
            ));
        }

        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would transpose {} note(s) in measures \
                     {}-{} of track {} by {} semitone(s), same strings. To apply, call again \
                     with confirm=true and expected_revision={}.",
                    note_count, from, to, track_number, p.semitones, range.revision,
                ),
                revision: range.revision,
                measures_added: None,
                notes_before: Some(note_count),
                notes_after: Some(note_count),
            }));
        }
        let expected_revision = p.expected_revision.ok_or_else(|| {
            ErrorData::invalid_params(
                "confirm=true requires expected_revision (from the preview call)",
                None,
            )
        })?;
        let result = self
            .call_bridge(move |client| {
                client.apply_replace_measures(track_number, from, &measures, expected_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied: transposed measures {}-{} of track {} by {} semitone(s). \
                 The user can undo with Cmd+Z.",
                from, to, track_number, p.semitones,
            ),
            revision: result.new_revision,
            measures_added: None,
            notes_before: Some(note_count),
            notes_after: Some(note_count),
        }))
    }

    #[tool(
        description = "Optimize fret positions of a monophonic passage: finds the lowest-effort string/fret path (dynamic programming over hand movement, position shifts, string skips, open strings) without changing any pitches. Defaults to the user's active selection. TWO-STEP SAFETY: preview (reports the effort improvement), then confirm=true with expected_revision. Fails on polyphonic passages (chords) in this version.",
        annotations(
            title = "Optimize fingering",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn tuxguitar_optimize_fingering(
        &self,
        params: Parameters<OptimizeFingeringParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        let (song, selection) = self
            .call_bridge(|client| {
                let song = client.read_song()?;
                let selection = client.read_selection()?;
                Ok((song, selection))
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;

        let track_number = p
            .track_number
            .or(if selection.active {
                selection.track_number
            } else {
                None
            })
            .unwrap_or(1);
        let song_len = song.headers.len() as u32;
        let (from, to) = match (p.from_measure, p.to_measure) {
            (Some(from), Some(to)) => (from, to),
            (Some(from), None) => (from, from),
            (None, _) if selection.active => (
                selection.from_measure.unwrap_or(1),
                selection.to_measure.unwrap_or(song_len),
            ),
            _ => (1, song_len.max(1)),
        };
        if from == 0 || to < from || to > song_len {
            return Err(ErrorData::invalid_params(
                format!("invalid measure range {from}-{to}: the score has measures 1-{song_len}"),
                None,
            ));
        }
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track_number} does not exist"), None)
            })?;
        if track.is_percussion {
            return Err(ErrorData::invalid_params(
                "cannot optimize fingering on a percussion track",
                None,
            ));
        }
        let tuning: Vec<(u32, u8)> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();
        let open_by_string: std::collections::HashMap<u32, u8> = tuning.iter().copied().collect();
        let max_fret = if track.max_fret > 0 {
            track.max_fret
        } else {
            24
        };

        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let mut measures = range.measures;

        // Collect the monophonic pitch sequence + current positions.
        let mut pitches: Vec<u8> = Vec::new();
        let mut old_positions: Vec<tabmcp_theory::fingering::Position> = Vec::new();
        for measure in &measures {
            for beat in &measure.beats {
                let sounding: Vec<&tabmcp_model::Note> = beat
                    .voices
                    .iter()
                    .flat_map(|v| &v.notes)
                    .filter(|n| !n.tied)
                    .collect();
                if sounding.len() > 1 {
                    return Err(ErrorData::invalid_params(
                        format!(
                            "measure {}, tick {}: {} simultaneous notes — this version \
                             optimizes monophonic lines only; narrow the range to a \
                             single-note passage",
                            measure.number,
                            beat.start_tick,
                            sounding.len()
                        ),
                        None,
                    ));
                }
                if let Some(note) = sounding.first() {
                    let open = open_by_string.get(&note.string).copied().ok_or_else(|| {
                        ErrorData::internal_error(
                            format!("note on unknown string {}", note.string),
                            None,
                        )
                    })?;
                    pitches.push(open.saturating_add(note.fret as u8));
                    old_positions.push(tabmcp_theory::fingering::Position {
                        string_number: note.string,
                        fret: note.fret,
                    });
                }
            }
        }
        if pitches.is_empty() {
            return Err(ErrorData::invalid_params(
                "the selected passage contains no notes to optimize",
                None,
            ));
        }

        let mut model = tabmcp_theory::fingering::CostModel::default();
        if p.min_fret.is_some() || p.max_fret_limit.is_some() {
            model.fret_range = Some((
                p.min_fret.unwrap_or(0),
                p.max_fret_limit.unwrap_or(max_fret).min(max_fret),
            ));
        }
        let optimized =
            tabmcp_theory::fingering::optimize_monophonic(&pitches, &tuning, max_fret, &model)
                .map_err(|indices| {
                    ErrorData::invalid_params(
                        format!(
                            "{} note(s) not playable within the given constraints \
                             (tuning/fret range)",
                            indices.len()
                        ),
                        None,
                    )
                })?;
        let old_breakdown = tabmcp_theory::fingering::breakdown(&old_positions, &model);
        let new_breakdown = tabmcp_theory::fingering::breakdown(&optimized.path, &model);
        let reasons = tabmcp_theory::fingering::explain_improvement(&old_breakdown, &new_breakdown)
            .join("; ");
        let old_cost = old_breakdown.total_cost;
        let changed = optimized
            .path
            .iter()
            .zip(&old_positions)
            .filter(|(new, old)| new != old)
            .count();

        // Write the optimized positions back into the measures.
        let mut cursor = 0usize;
        let mut last_position: Option<tabmcp_theory::fingering::Position> = None;
        for measure in &mut measures {
            for beat in &mut measure.beats {
                for voice in &mut beat.voices {
                    for note in &mut voice.notes {
                        let position = if note.tied {
                            last_position // a tie continues on the same string
                        } else {
                            let p = optimized.path.get(cursor).copied();
                            cursor += 1;
                            last_position = p;
                            p
                        };
                        if let Some(p) = position {
                            note.string = p.string_number;
                            note.fret = p.fret;
                        }
                    }
                }
            }
        }

        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would re-finger {changed} of {} notes in \
                     measures {from}-{to} of track {track_number}; hand-effort cost {:.1} -> \
                     {:.1} (lower is easier). Why: {reasons}. All pitches stay identical. To \
                     apply, call again with confirm=true and expected_revision={}.",
                    pitches.len(),
                    old_cost,
                    optimized.cost,
                    range.revision,
                ),
                revision: range.revision,
                measures_added: None,
                notes_before: Some(pitches.len() as u32),
                notes_after: Some(pitches.len() as u32),
            }));
        }
        let expected_revision = p.expected_revision.ok_or_else(|| {
            ErrorData::invalid_params(
                "confirm=true requires expected_revision (from the preview call)",
                None,
            )
        })?;
        let result = self
            .call_bridge(move |client| {
                client.apply_replace_measures(track_number, from, &measures, expected_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied: re-fingered {changed} of {} notes in measures {from}-{to} of track \
                 {track_number} (hand-effort {:.1} -> {:.1}); pitches unchanged. The user can \
                 undo with Cmd+Z.",
                pitches.len(),
                old_cost,
                optimized.cost,
            ),
            revision: result.new_revision,
            measures_added: None,
            notes_before: Some(pitches.len() as u32),
            notes_after: Some(pitches.len() as u32),
        }))
    }

    #[tool(
        description = "Generate a root-following bassline from a guitar passage: detects the harmony per measure, mirrors the source rhythm, adds chromatic approach notes into root changes, and writes it to a NEW 4-string bass track (fingered by the optimizer). Defaults to the selection. TWO-STEP: preview describes the line; confirm=true with expected_revision creates the track and writes it (undoable).",
        annotations(
            title = "Generate bassline",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_generate_bassline(
        &self,
        params: Parameters<GenerateParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        self.generate(params.0, GenerateKind::Bassline).await
    }

    #[tool(
        description = "Generate a diatonic harmony line (3rds or 6ths above the lead, staying in the detected scale) from a monophonic passage, written to a NEW track with the same tuning as the source (fingered by the optimizer). Defaults to the selection. TWO-STEP: preview first, then confirm=true with expected_revision (undoable).",
        annotations(
            title = "Generate harmony",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_generate_harmony(
        &self,
        params: Parameters<GenerateParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        self.generate(params.0, GenerateKind::Harmony).await
    }

    #[tool(
        description = "Generate a basic rock/metal drum part locked to the source passage's accents: closed hi-hat eighths, snare backbeats, kick doubling the guitar's low-register hits (plus the downbeat). Written to a NEW percussion track. Defaults to the selection. TWO-STEP: preview first, then confirm=true with expected_revision (undoable).",
        annotations(
            title = "Generate drums",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_generate_drums(
        &self,
        params: Parameters<GenerateParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        self.generate(params.0, GenerateKind::Drums).await
    }

    #[tool(
        description = "Create a new track in the open score with a name and tuning (explicit note names or a preset like '7-string A standard'). The new track is appended after the existing ones and is undoable.",
        annotations(
            title = "Create track",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_create_track(
        &self,
        params: Parameters<CreateTrackParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let strings = resolve_tuning(&p.tuning, &p.preset)?;
        let names = tuning_names(&strings);
        let name = p.name.clone();
        let clef = p.clef.clone();
        let percussion = p.percussion;
        let result = self
            .call_bridge(move |client| {
                client.create_track(&name, &strings, clef.as_deref(), percussion)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let track_number = result
            .get("trackNumber")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        Ok(format!(
            "Created track {} \"{}\" with {}-string tuning: {} (high to low). \
             Write to it with tuxguitar_replace_measures using track_number={}.",
            track_number,
            p.name,
            names.split(' ').count(),
            names,
            track_number,
        ))
    }

    #[tool(
        description = "Change the tuning of an existing track (explicit note names or a preset like '7-string A standard'; this also changes the string count). TWO-STEP SAFETY: preview first, then confirm=true with expected_revision. Undoable.",
        annotations(
            title = "Change tuning",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn tuxguitar_change_tuning(
        &self,
        params: Parameters<ChangeTuningParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        let strings = resolve_tuning(&p.tuning, &p.preset)?;
        let names = tuning_names(&strings);
        let song = self.fetch_song().await?;
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == p.track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {} does not exist", p.track_number), None)
            })?;

        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would retune track {} (\"{}\") from [{}] \
                     to [{}] ({} strings). Existing notes keep their fret numbers, so sounding \
                     pitches shift with the tuning. To apply, call again with confirm=true and \
                     expected_revision={}.",
                    p.track_number,
                    track.name,
                    tuning_names(&track.strings),
                    names,
                    strings.len(),
                    song.revision,
                ),
                revision: song.revision,
                measures_added: None,
                notes_before: None,
                notes_after: None,
            }));
        }
        let expected_revision = p.expected_revision.ok_or_else(|| {
            ErrorData::invalid_params(
                "confirm=true requires expected_revision (from the preview call)",
                None,
            )
        })?;
        let result = self
            .call_bridge(move |client| {
                client.change_tuning(p.track_number, &strings, expected_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied: track {} retuned to [{}]. The user can undo with Cmd+Z.",
                p.track_number, names,
            ),
            revision: result
                .get("newRevision")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            measures_added: None,
            notes_before: None,
            notes_after: None,
        }))
    }

    #[tool(
        description = "The 'virtual ear': listen to the whole arrangement the way a producer reads a session. Analyzes ALL tracks together over a measure range (default: whole song, max 32 measures) and reports harsh cross-track dissonances (minor 2nds / tritones at exact measure+tick), register overlaps that cause masking, rhythmic tightness between parts, empty/sparse measures, and velocity balance. Use it after composing/generating to hear problems, then fix them with the edit tools.",
        annotations(title = "Analyze arrangement", read_only_hint = true)
    )]
    async fn tuxguitar_analyze_arrangement(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let song = self.fetch_song().await?;
        let song_len = song.headers.len() as u32;
        let from = p.from_measure.unwrap_or(1);
        let to = p
            .to_measure
            .unwrap_or(song_len)
            .min(from + MAX_MEASURES_PER_READ - 1);
        if from == 0 || to < from || to > song_len {
            return Err(ErrorData::invalid_params(
                format!("invalid measure range {from}-{to}: the score has measures 1-{song_len}"),
                None,
            ));
        }

        let mut inputs = Vec::new();
        for track in &song.tracks {
            let track_number = track.number;
            let range = self
                .call_bridge(move |client| client.read_measures(track_number, from, to))
                .await
                .map_err(BridgeCallError::into_error_data)?;
            inputs.push(tabmcp_theory::arrangement::TrackInput {
                number: track.number,
                name: track.name.clone(),
                is_percussion: track.is_percussion,
                tuning: track
                    .strings
                    .iter()
                    .map(|s| (s.number, s.open_pitch))
                    .collect(),
                measures: range.measures,
            });
        }
        let report = tabmcp_theory::arrangement::analyze_arrangement(&inputs);
        Ok(format!(
            "Arrangement analysis, measures {from}-{to}, {} track(s):\n\n{}",
            inputs.len(),
            tabmcp_theory::arrangement::describe(&report),
        ))
    }

    #[tool(
        description = "Change the tempo (BPM): the whole song by default, or from a given measure to the end (for tempo ramps, call once per section). NOTE: tempo changes are not in TuxGuitar's undo stack (the app's own tempo dialog isn't undoable either) — call again with the old BPM to revert.",
        annotations(title = "Set tempo", read_only_hint = false, destructive_hint = false)
    )]
    async fn tuxguitar_set_tempo(
        &self,
        params: Parameters<SetTempoParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        if p.bpm < 1 || p.bpm > 320 {
            return Err(ErrorData::invalid_params("bpm must be 1..320", None));
        }
        self.call_bridge(move |client| client.set_tempo(p.bpm, p.from_measure))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(match p.from_measure {
            Some(measure) => format!("Tempo set to {} BPM from measure {measure} onward.", p.bpm),
            None => format!("Tempo set to {} BPM for the whole song.", p.bpm),
        })
    }

    #[tool(
        description = "Export the score via TuxGuitar's own writers — opens the export dialog pre-set to the format (default 'mid' = one multitrack MIDI file with all tracks, drums on channel 10, repeats expanded); the user picks the file name and location. Great for handing the arrangement to a DAW.",
        annotations(title = "Export", read_only_hint = true)
    )]
    async fn tuxguitar_export(
        &self,
        params: Parameters<ExportParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let format = p.format.unwrap_or_else(|| "mid".into());
        let result = self
            .call_bridge(move |client| client.export_song(&format))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(format!(
            "Export dialog opened in TuxGuitar ({} format) — the user picks the destination.",
            result
                .get("format")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("requested")
        ))
    }

    #[tool(
        description = "Set repeat signs so a measure range loops during playback (and in MIDI export): repeat-open at from_measure, repeat-close at to_measure with the given repeat count. repetitions=0 clears the repeat. Undoable.",
        annotations(
            title = "Set repeat/loop",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_set_repeat(
        &self,
        params: Parameters<SetRepeatParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let repetitions = p.repetitions.unwrap_or(2);
        self.call_bridge(move |client| {
            client.set_repeat(p.from_measure, p.to_measure, repetitions)
        })
        .await
        .map_err(BridgeCallError::into_error_data)?;
        Ok(if repetitions == 0 {
            format!(
                "Repeat cleared on measures {}-{}.",
                p.from_measure, p.to_measure
            )
        } else {
            format!(
                "Measures {}-{} now repeat {} time(s) during playback — press play to loop.",
                p.from_measure, p.to_measure, repetitions
            )
        })
    }

    #[tool(
        description = "Start playback in TuxGuitar from the current cursor position (acts like the play button; calling while playing pauses). Use tuxguitar_stop to stop.",
        annotations(title = "Play", read_only_hint = false, destructive_hint = false)
    )]
    async fn tuxguitar_play(&self) -> Result<String, ErrorData> {
        self.call_bridge(|client| client.play())
            .await
            .map(|_| "Playback toggled in TuxGuitar.".to_string())
            .map_err(BridgeCallError::into_error_data)
    }

    #[tool(
        description = "Stop playback in TuxGuitar and return the cursor to where playback started.",
        annotations(title = "Stop", read_only_hint = false, destructive_hint = false)
    )]
    async fn tuxguitar_stop(&self) -> Result<String, ErrorData> {
        self.call_bridge(|client| client.stop())
            .await
            .map(|_| "Playback stopped.".to_string())
            .map_err(BridgeCallError::into_error_data)
    }

    #[tool(
        description = "Open TuxGuitar's Save-As dialog so the user can save the current score (e.g. as a copy before/after AI edits). The user chooses the filename and location themselves.",
        annotations(title = "Save a copy", read_only_hint = true)
    )]
    async fn tuxguitar_save_copy(&self) -> Result<String, ErrorData> {
        self.call_bridge(|client| client.save_copy())
            .await
            .map(|result| {
                if result.dialog_opened {
                    "TuxGuitar's Save-As dialog is now open — the user picks the file name \
                     and location."
                        .to_string()
                } else {
                    "Save-As could not be opened.".to_string()
                }
            })
            .map_err(BridgeCallError::into_error_data)
    }
}

// ---------- bridge plumbing ----------

/// Error from a bridge call, with an actionable message for the model.
struct BridgeCallError {
    message: String,
}

impl BridgeCallError {
    fn into_error_data(self) -> ErrorData {
        ErrorData::internal_error(self.message, None)
    }
}

fn actionable(error: &BridgeError) -> String {
    match error {
        BridgeError::NotRunning(_) | BridgeError::Unreachable { .. } => format!(
            "{error}. Ask the user to start TuxGuitar (with the TabMCP bridge plugin installed), \
             then retry."
        ),
        BridgeError::Rejected { code, message } if code == "E_NO_DOCUMENT" => {
            format!("{message}. Ask the user to open or create a score in TuxGuitar, then retry.")
        }
        _ => error.to_string(),
    }
}

#[derive(Clone, Copy)]
enum GenerateKind {
    Bassline,
    Harmony,
    Drums,
}

impl TabMcp {
    /// Shared driver for the generation tools: resolve scope, read source,
    /// generate, then (on confirm) create the target track and write.
    async fn generate(
        &self,
        p: GenerateParams,
        kind: GenerateKind,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let (song, selection) = self
            .call_bridge(|client| {
                let song = client.read_song()?;
                let selection = client.read_selection()?;
                Ok((song, selection))
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;

        let source_track = p
            .source_track
            .or(if selection.active {
                selection.track_number
            } else {
                None
            })
            .unwrap_or(1);
        let song_len = song.headers.len() as u32;
        let (from, to) = match (p.from_measure, p.to_measure) {
            (Some(from), Some(to)) => (from, to),
            (Some(from), None) => (from, from),
            (None, _) if selection.active => (
                selection.from_measure.unwrap_or(1),
                selection.to_measure.unwrap_or(song_len),
            ),
            _ => (1, song_len.max(1)),
        };
        if from == 0 || to < from || to > song_len {
            return Err(ErrorData::invalid_params(
                format!("invalid measure range {from}-{to}: the score has measures 1-{song_len}"),
                None,
            ));
        }
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == source_track)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {source_track} does not exist"), None)
            })?;
        let source_tuning: Vec<(u32, u8)> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();
        let max_fret = if track.max_fret > 0 {
            track.max_fret
        } else {
            24
        };

        let range = self
            .call_bridge(move |client| client.read_measures(source_track, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;

        // Generate.
        let bass_tuning: Vec<(u32, u8)> = tabmcp_theory::tuning_preset("4-string bass")
            .expect("preset exists")
            .iter()
            .enumerate()
            .map(|(i, &pitch)| (i as u32 + 1, pitch))
            .collect();
        let interval = p.interval.clone().unwrap_or_else(|| "third".into());
        let (new_track_name, target_strings, generated, description, clef) = match kind {
            GenerateKind::Bassline => {
                let (measures, description) = tabmcp_theory::generation::generate_bassline(
                    &range.measures,
                    &source_tuning,
                    &bass_tuning,
                    24,
                )
                .map_err(|e| ErrorData::invalid_params(e, None))?;
                let strings: Vec<tabmcp_model::StringTuning> = bass_tuning
                    .iter()
                    .map(|&(number, open_pitch)| tabmcp_model::StringTuning { number, open_pitch })
                    .collect();
                (
                    "Bass (AI)".to_string(),
                    strings,
                    measures,
                    description,
                    Some("bass"),
                )
            }
            GenerateKind::Harmony => {
                let (measures, description) = tabmcp_theory::generation::generate_harmony(
                    &range.measures,
                    &source_tuning,
                    max_fret,
                    &interval,
                )
                .map_err(|e| ErrorData::invalid_params(e, None))?;
                let strings: Vec<tabmcp_model::StringTuning> = track.strings.clone();
                (
                    "Harmony Guitar (AI)".to_string(),
                    strings,
                    measures,
                    description,
                    None::<&str>,
                )
            }
            GenerateKind::Drums => {
                let (measures, description) =
                    tabmcp_theory::generation::generate_drums(&range.measures, &source_tuning)
                        .map_err(|e| ErrorData::invalid_params(e, None))?;
                // Percussion strings are tuned to 0 so fret == drum key.
                let strings: Vec<tabmcp_model::StringTuning> = (1..=6)
                    .map(|number| tabmcp_model::StringTuning {
                        number,
                        open_pitch: 0,
                    })
                    .collect();
                (
                    "Drums (AI)".to_string(),
                    strings,
                    measures,
                    description,
                    None::<&str>,
                )
            }
        };
        let percussion = matches!(kind, GenerateKind::Drums);
        let note_count = count_notes(&generated);

        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would create a new track \"{new_track_name}\" \
                     and write {note_count} notes into measures {from}-{to}: {description}. \
                     Source: track {source_track} (\"{}\"). To apply, call again with \
                     confirm=true and expected_revision={}. (Undoing afterwards takes two \
                     Cmd+Z steps: one for the notes, one for the track.)",
                    track.name, song.revision,
                ),
                revision: song.revision,
                measures_added: None,
                notes_before: Some(0),
                notes_after: Some(note_count),
            }));
        }
        let expected_revision = p.expected_revision.ok_or_else(|| {
            ErrorData::invalid_params(
                "confirm=true requires expected_revision (from the preview call)",
                None,
            )
        })?;
        // Stale check before creating the track (creation itself bumps the
        // revision, so the write below uses the post-creation revision).
        let ping = self
            .call_bridge(|client| client.ping())
            .await
            .map_err(BridgeCallError::into_error_data)?;
        if ping.revision != expected_revision {
            return Err(ErrorData::invalid_params(
                format!(
                    "score changed: expected revision {expected_revision}, current is {} — \
                     re-run the preview",
                    ping.revision
                ),
                None,
            ));
        }

        let name_for_create = new_track_name.clone();
        let created = self
            .call_bridge(move |client| {
                client.create_track(&name_for_create, &target_strings, clef, percussion)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let new_track = created
            .get("trackNumber")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let post_create_revision = created
            .get("newRevision")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        let result = self
            .call_bridge(move |client| {
                client.apply_replace_measures(new_track, from, &generated, post_create_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied: created track {new_track} \"{new_track_name}\" and wrote {} notes \
                 into measures {from}-{to} — {description}. Undo takes two Cmd+Z steps.",
                result.notes_after,
            ),
            revision: result.new_revision,
            measures_added: Some(0),
            notes_before: Some(0),
            notes_after: Some(result.notes_after),
        }))
    }

    /// Run a bridge operation on a blocking thread, connecting (or
    /// reconnecting once) as needed.
    async fn call_bridge<T, F>(&self, operation: F) -> Result<T, BridgeCallError>
    where
        T: Send + 'static,
        F: FnOnce(&mut BridgeClient) -> Result<T, BridgeError> + Send + 'static,
    {
        let bridge = Arc::clone(&self.bridge);
        let path = self.discovery_path.clone();
        tokio::task::spawn_blocking(move || {
            let mut slot = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if slot.is_none() {
                *slot = Some(BridgeClient::connect(&path).map_err(|e| BridgeCallError {
                    message: actionable(&e),
                })?);
            }
            let client = slot.as_mut().expect("client just ensured");
            match operation(client) {
                Ok(value) => Ok(value),
                Err(error) => {
                    // On transport-level failures the connection is dead;
                    // drop it so the next call reconnects cleanly.
                    if matches!(error, BridgeError::Io(_) | BridgeError::Malformed(_)) {
                        *slot = None;
                    }
                    Err(BridgeCallError {
                        message: actionable(&error),
                    })
                }
            }
        })
        .await
        .map_err(|join_error| BridgeCallError {
            message: format!("internal task failure: {join_error}"),
        })?
    }

    async fn fetch_song(&self) -> Result<Song, ErrorData> {
        self.call_bridge(|client| client.read_song())
            .await
            .map_err(BridgeCallError::into_error_data)
    }

    /// Resolve the analysis scope (explicit args > selection > whole track 1)
    /// and flatten the passage into ordered note events.
    async fn collect_events(
        &self,
        params: AnalysisScopeParams,
    ) -> Result<(String, Vec<NoteEvent>), ErrorData> {
        let (song, selection) = self
            .call_bridge(|client| {
                let song = client.read_song()?;
                let selection = client.read_selection()?;
                Ok((song, selection))
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;

        let track_number = params
            .track_number
            .or(if selection.active {
                selection.track_number
            } else {
                None
            })
            .unwrap_or(1);
        let last_measure = song.headers.len() as u32;
        let (from, to) = match (params.from_measure, params.to_measure) {
            (Some(from), Some(to)) => (from, to),
            (Some(from), None) => (from, from),
            (None, _) if selection.active => (
                selection.from_measure.unwrap_or(1),
                selection.to_measure.unwrap_or(last_measure),
            ),
            _ => (1, last_measure.max(1)),
        };
        if from == 0 || to < from || to > last_measure {
            return Err(ErrorData::invalid_params(
                format!(
                    "invalid measure range {from}-{to}: the score has measures 1-{last_measure}"
                ),
                None,
            ));
        }

        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    format!(
                        "track {track_number} does not exist: the score has tracks {}",
                        song.tracks
                            .iter()
                            .map(|t| t.number.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    None,
                )
            })?;
        let open_pitch: std::collections::HashMap<u32, u8> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();

        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;

        let mut events = Vec::new();
        for measure in &range.measures {
            for beat in &measure.beats {
                for voice in &beat.voices {
                    let ticks = duration_ticks(&voice.duration);
                    for note in &voice.notes {
                        if note.tied {
                            continue; // continuation of the previous event
                        }
                        if let Some(&open) = open_pitch.get(&note.string) {
                            events.push(NoteEvent {
                                pitch: open.saturating_add(note.fret as u8),
                                weight: ticks,
                            });
                        }
                    }
                }
            }
        }
        let scope = format!(
            "track {track_number} ({}), measures {from}-{to}",
            track.name
        );
        Ok((scope, events))
    }
}

fn duration_ticks(duration: &tabmcp_model::Duration) -> u64 {
    let mut ticks = TICKS_PER_QUARTER * 4 / duration.value.max(1) as u64;
    if duration.dotted {
        ticks = ticks * 3 / 2;
    } else if duration.double_dotted {
        ticks = ticks * 7 / 4;
    }
    if !duration.tuplet.is_normal() && duration.tuplet.enters > 0 {
        ticks = ticks * duration.tuplet.times as u64 / duration.tuplet.enters as u64;
    }
    ticks.max(1)
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TabMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "TabMCP connects you to the score currently open in the TuxGuitar tablature \
                 editor. Typical flow: tuxguitar_get_score_summary for orientation, \
                 tuxguitar_get_selection to see what the user highlighted, then \
                 tuxguitar_get_measures / tuxguitar_explain_selection / \
                 tuxguitar_detect_key_and_scale for content and analysis. If tools report the \
                 bridge is unavailable, the user needs to start TuxGuitar with the TabMCP \
                 plugin installed. String numbers are 1-based (1 = highest string); pitch = \
                 open-string pitch + fret; time is in ticks (960 per quarter note)."
                .into(),
        );
        info
    }
}
