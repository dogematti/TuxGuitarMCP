//! The MCP server: exposes the TuxGuitar bridge and the theory engine as
//! MCP tools over stdio.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorData, PromptMessage, Role, ServerCapabilities, ServerInfo};
use rmcp::{
    prompt, prompt_handler, prompt_router, tool, tool_handler, tool_router, ServerHandler,
    ServiceExt,
};
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
    /// AI Ear pass history for this session: (mean groove, issue count) per
    /// evaluate call, so the loop can report trends across passes.
    passes: Arc<std::sync::Mutex<Vec<(f64, usize)>>>,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

#[derive(serde::Deserialize, JsonSchema)]
struct ComposePromptArgs {
    /// Style, e.g. "groove metal", "blues shuffle".
    style: String,
    /// Key/scale, e.g. "A phrygian dominant", "E minor".
    key: String,
    /// Number of bars (as text, e.g. "8").
    #[serde(default)]
    bars: Option<String>,
}

#[derive(serde::Deserialize, JsonSchema)]
struct RefinePromptArgs {
    /// How many refinement passes to run (as text, default "3").
    #[serde(default)]
    passes: Option<String>,
}

#[prompt_router]
impl TabMcp {
    /// Compose a full arrangement from scratch and refine it.
    #[prompt(
        name = "compose",
        description = "Compose a riff + bass + drums in a style and key, then refine with the AI Ear loop."
    )]
    async fn compose_prompt(&self, params: Parameters<ComposePromptArgs>) -> Vec<PromptMessage> {
        let bars = params.0.bars.unwrap_or_else(|| "8".into());
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Using the tuxguitar tools: compose a {bars}-bar {} riff in {} on the current \
                 track (create/retune tracks as needed), generate bass and drums from it \
                 (pick a fitting drum style), loop it, then run the AI-Ear refinement loop \
                 (tuxguitar_evaluate -> fix top issue -> re-evaluate) until the scorecard is \
                 clean, humanize, and finish with tuxguitar_render_and_listen plus a summary \
                 of every pass and the Cmd+Z rollback map.",
                params.0.style, params.0.key,
            ),
        )]
    }

    /// Review the open score as five band personalities, then fix by vote.
    #[prompt(
        name = "band",
        description = "Band review: five personalities (composer, critic, producer, guitarist, listener) each judge the open score with their own tools, vote, and the top-voted fixes get applied."
    )]
    async fn band_prompt(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            "Review the score currently open in TuxGuitar as five distinct band \
             personalities, each using its own tools, each speaking in its own voice:\n\
             1. THE COMPOSER - tuxguitar_track_themes and tuxguitar_riff_dna: do motifs \
             develop across sections or does the song forget its own material?\n\
             2. THE CRITIC - tuxguitar_evaluate and tuxguitar_hook_check: is anything \
             here memorable? Reject freely; the critic is never polite.\n\
             3. THE PRODUCER - tuxguitar_producer_notes and tuxguitar_style_match: does \
             the arrangement build, drop, and land where it should?\n\
             4. THE GUITARIST - tuxguitar_check_realism and tuxguitar_analyze_difficulty: \
             can hands actually play this for a full take?\n\
             5. THE LISTENER - tuxguitar_render_and_listen and tuxguitar_listen_stems: \
             does the actual audio slap? Any instrument buried or boring?\n\
             Then hold the vote: each personality names the ONE change it wants most. \
             Apply the changes that get 2+ votes (preview -> confirm, all undoable), \
             re-run tuxguitar_evaluate to confirm improvement, and report the band \
             meeting minutes: who said what, what won the vote, what changed.",
        )]
    }

    /// Run the AI-Ear refinement loop on the current score.
    #[prompt(
        name = "refine",
        description = "Run N passes of the AI Ear loop (evaluate -> fix -> re-listen) on the open score."
    )]
    async fn refine_prompt(&self, params: Parameters<RefinePromptArgs>) -> Vec<PromptMessage> {
        let passes = params.0.passes.unwrap_or_else(|| "3".into());
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Run {passes} refinement pass(es) on the score currently open in TuxGuitar: \
                 each pass call tuxguitar_evaluate and tuxguitar_listen_stems, fix the top \
                 issue with the edit tools (preview -> confirm), and explain what changed and \
                 why. Finish with tuxguitar_render_and_listen and the Cmd+Z rollback map.",
            ),
        )]
    }
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
    /// Cost preset: "default" or "metal" (low compact positions, open-string
    /// chug preference, stronger shift penalty).
    #[serde(default)]
    cost_preset: Option<String>,
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
    /// Drums only: groove style — "rock" (default), "metal-gallop",
    /// "punk", or "halftime".
    #[serde(default)]
    style: Option<String>,
    /// Write into this EXISTING track instead of creating a new one —
    /// enables per-section generation (e.g. gallop drums for bars 1-8,
    /// halftime for the breakdown) by calling per range.
    #[serde(default)]
    target_track: Option<u32>,
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
struct PlayFromParams {
    /// Measure to start playback from (1-based).
    measure: u32,
}

#[derive(Deserialize, JsonSchema)]
struct HumanizeParams {
    /// Track number (1-based). Omit to use the active selection's track.
    #[serde(default)]
    track_number: Option<u32>,
    /// First measure. Omit to use the selection, falling back to the whole track.
    #[serde(default)]
    from_measure: Option<u32>,
    /// Last measure (inclusive).
    #[serde(default)]
    to_measure: Option<u32>,
    /// Maximum velocity variation (default 8, max 30).
    #[serde(default)]
    amount: Option<u32>,
    /// False (default): preview only. True: apply — requires expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct ImportMidiParams {
    /// Name for the new track (default "Imported (AI)").
    #[serde(default)]
    track_name: Option<String>,
    /// Tuning preset for the target track (default "6-string standard").
    #[serde(default)]
    preset: Option<String>,
    /// Quantization grid denominator: 8, 16 (default) or 32.
    #[serde(default)]
    quantize: Option<u32>,
    /// Which MIDI content track to import (1-based). Default: the densest.
    #[serde(default)]
    midi_track: Option<usize>,
    /// False (default): preview. True: create the track and write.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct CopyMeasuresParams {
    /// Track to copy from (1-based).
    source_track: u32,
    /// First source measure (1-based).
    from_measure: u32,
    /// Last source measure (inclusive).
    to_measure: u32,
    /// Destination measure (1-based); content there is replaced.
    dest_measure: u32,
    /// Destination track (default: same as source).
    #[serde(default)]
    dest_track: Option<u32>,
    /// False (default): preview. True: apply - requires expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct SetTimeSignatureParams {
    /// Measure where the new time signature starts (1-based).
    measure: u32,
    /// Beats per measure (1..32).
    numerator: u32,
    /// Beat unit: 4 = quarter, 8 = eighth, ...
    denominator: u32,
    /// Apply to the end of the song (default true) or this measure only.
    #[serde(default)]
    to_end: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct SetKeySignatureParams {
    /// Measure where the key starts (1-based).
    measure: u32,
    /// TuxGuitar key code: 0 = C/Am; 1..7 = sharps; 8..14 = flats (1b..7b).
    key: i32,
    /// Track number (default 1).
    #[serde(default)]
    track_number: Option<u32>,
    /// Apply to the end (default true).
    #[serde(default)]
    to_end: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct InsertMeasuresParams {
    /// Position (1-based): insert before / delete starting at this measure.
    at: u32,
    /// How many measures (default 1).
    #[serde(default)]
    count: Option<u32>,
}

#[derive(Default, Deserialize, JsonSchema)]
struct StyleGuideParams {
    /// Style name (e.g. "djent", "doom", "blues rock"). Omit to list all.
    #[serde(default)]
    style: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct VaryRiffParams {
    /// Track (1-based).
    track_number: u32,
    /// Source range (1-based, inclusive).
    from_measure: u32,
    to_measure: u32,
    /// One of: "displace", "retrograde", "invert", "octave", "augment",
    /// "diminish", "pedal", "regroup", "swap_dynamics".
    transform: String,
    /// displace: shift in ticks (default 480 = an 8th; odd amounts like 120
    /// or 360 give polyrhythmic feels). pedal/regroup: grid unit in ticks
    /// (default 240 = a 16th).
    #[serde(default)]
    amount: Option<u32>,
    /// octave only: how many octaves to shift, negative = down (default 1).
    #[serde(default)]
    octaves: Option<i32>,
    /// regroup only: accent grouping like "3+3+2" (units of `amount` ticks),
    /// cycling across barlines for implied polymeter.
    #[serde(default)]
    grouping: Option<String>,
    /// pedal only: string/fret of the pedal tone (default: lowest string open).
    #[serde(default)]
    pedal_string: Option<u32>,
    #[serde(default)]
    pedal_fret: Option<u32>,
    /// Where to write the result (default: in place at from_measure).
    #[serde(default)]
    dest_measure: Option<u32>,
    /// False (default): preview. True: apply - requires expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Default, Deserialize, JsonSchema)]
struct EvaluateParams {
    /// First measure to analyze (1-based, default 1).
    #[serde(default)]
    from_measure: Option<u32>,
    /// Last measure (inclusive, default: whole song).
    #[serde(default)]
    to_measure: Option<u32>,
    /// Style name (from tuxguitar_style_guide) to check tempo and
    /// syncopation against that style's targets.
    #[serde(default)]
    style: Option<String>,
    /// Target tension curve as comma-separated 0..1 values (e.g.
    /// "0.2,0.5,0.8,1.0,0.4") — the measured curve is compared against it.
    #[serde(default)]
    tension_target: Option<String>,
    /// Emotional journey as comma- or arrow-separated feeling words (e.g.
    /// "calm, uneasy, aggressive, victorious") — mapped onto the tension
    /// scale and compared like tension_target. Vocabulary: calm, dreamy,
    /// mournful, hopeless, brooding, uneasy, menacing, tense, driving,
    /// epic, aggressive, triumphant, victorious, furious.
    #[serde(default)]
    emotion_target: Option<String>,
}

#[derive(Default, Deserialize, JsonSchema)]
struct RiffDnaParams {
    /// Track (1-based). Required unless list_bank.
    #[serde(default)]
    track_number: Option<u32>,
    /// Source riff range (1-based, inclusive). Required unless list_bank.
    #[serde(default)]
    from_measure: Option<u32>,
    #[serde(default)]
    to_measure: Option<u32>,
    /// Save the extracted DNA under this name in the local DNA bank
    /// (~/.tuxguitar-mcp/dna_bank.jsonl) for later sessions.
    #[serde(default)]
    save_as: Option<String>,
    /// True: list the saved DNA bank instead of extracting.
    #[serde(default)]
    list_bank: bool,
}

#[derive(Deserialize, JsonSchema)]
struct HookCheckParams {
    /// Track (1-based).
    track_number: u32,
    /// Riff range (1-based, inclusive).
    from_measure: u32,
    to_measure: u32,
}

#[derive(Deserialize, JsonSchema)]
struct GenerateRiffParams {
    /// Track to write into (1-based); its tuning defines playability.
    track_number: u32,
    /// Destination range (1-based, inclusive) - its meters define the bars.
    from_measure: u32,
    to_measure: u32,
    /// Scale with root, e.g. "A phrygian dominant", "E harmonic minor".
    scale: String,
    /// Register bounds as note names (e.g. "A1".."D3"). Defaults to the
    /// track's lowest open string up to a 12th above it.
    #[serde(default)]
    register_low: Option<String>,
    #[serde(default)]
    register_high: Option<String>,
    /// Comma list of rhythm-cell names to build from (gallop, tresillo,
    /// herta, hemiola, quintuplet, ...). Omit for the default metal set.
    #[serde(default)]
    cells: Option<String>,
    /// Accent offsets in ticks within each measure (comma list; 960 per
    /// quarter). These get roots + accent marks - set them to your kick
    /// pattern for instant unison. Default "0".
    #[serde(default)]
    accents: Option<String>,
    /// Syncopation window like "0.2-0.6" (default 0.15-0.55).
    #[serde(default)]
    syncopation: Option<String>,
    /// Onsets per measure like "4-10" (default).
    #[serde(default)]
    density: Option<String>,
    /// Palm-mute the unaccented low notes (default true).
    #[serde(default)]
    palm_mute_low: Option<bool>,
    /// False (default): preview. True: apply - requires expected_revision.
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct RebarParams {
    /// Track (1-based).
    track_number: u32,
    /// Source material range (1-based, inclusive).
    from_measure: u32,
    to_measure: u32,
    /// Destination range whose measures already carry the DESIRED time
    /// signatures (set them first with tuxguitar_set_time_signature).
    dest_from: u32,
    dest_to: u32,
    /// False (default): preview. True: apply - requires expected_revision.
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct EvolveRiffParams {
    /// Track (1-based).
    track_number: u32,
    /// Seed riff range (1-based, inclusive).
    from_measure: u32,
    to_measure: u32,
    /// How many generations to evolve (default 4, max 8).
    #[serde(default)]
    generations: Option<u32>,
    /// Where to write the winner (default: in place at from_measure).
    #[serde(default)]
    dest_measure: Option<u32>,
    /// False (default): preview the lineage. True: apply - requires
    /// expected_revision.
    #[serde(default)]
    confirm: bool,
    /// The revision returned by the preview call.
    #[serde(default)]
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct SetMarkerParams {
    /// Measure to mark (1-based).
    measure: u32,
    /// Section name (e.g. "Verse", "Chorus"). Empty string clears the marker.
    title: String,
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
            passes: Arc::new(std::sync::Mutex::new(Vec::new())),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
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
        description = "Write tablature into the open score: replace a measure range on one track with new measures (notes as string+fret; measures past the end of the song are appended automatically). Effects per note: booleans (palmMute, vibrato, slide, hammer, letRing, staccato, deadNote, ...) plus parameterized harmonic {type: natural|artificial|tapped|pinch|semi} and bend {points: [{position 0-12, value in semitones}]} (empty points = standard full-tone bend). NOTE: palmMute, staccato and letRing are MUTUALLY EXCLUSIVE in TuxGuitar; if several are set, precedence is palmMute > staccato > letRing. TWO-STEP SAFETY: call without confirm to get a preview and revision, then call again with confirm=true and expected_revision to apply. The edit is atomic and undoable with Cmd+Z.",
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
        description = "Optimize string/fret choices of a passage — now CHORD-AWARE: single notes get the lowest-effort path, chords get re-voiced (compact playable voicings, unique strings) via the same candidates -> cost -> dynamic-programming search. Pitches never change. Defaults to the user's active selection. TWO-STEP SAFETY: preview (with effort delta), then confirm=true with expected_revision.",
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
        use tabmcp_theory::fingering::{optimize_steps, Position, Step};
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

        // Build steps: one per sounding beat — a Mono pitch or a Chord of
        // ascending pitches (chords are voiced by the optimizer too).
        let mut steps: Vec<Step> = Vec::new();
        let mut old_flat: Vec<Position> = Vec::new();
        let mut chord_count = 0usize;
        for measure in &measures {
            for beat in &measure.beats {
                let mut pitches: Vec<u8> = Vec::new();
                for voice in &beat.voices {
                    for note in &voice.notes {
                        if note.tied {
                            continue;
                        }
                        if let Some(&open) = open_by_string.get(&note.string) {
                            pitches.push(open.saturating_add(note.fret as u8));
                            old_flat.push(Position {
                                string_number: note.string,
                                fret: note.fret,
                            });
                        }
                    }
                }
                match pitches.len() {
                    0 => {}
                    1 => steps.push(Step::Mono(pitches[0])),
                    _ => {
                        pitches.sort_unstable();
                        chord_count += 1;
                        steps.push(Step::Chord(pitches));
                    }
                }
            }
        }
        if steps.is_empty() {
            return Err(ErrorData::invalid_params(
                "the selected passage contains no notes to optimize",
                None,
            ));
        }

        let mut model = p
            .cost_preset
            .as_deref()
            .and_then(tabmcp_theory::fingering::CostModel::preset)
            .unwrap_or_default();
        if p.min_fret.is_some() || p.max_fret_limit.is_some() {
            model.fret_range = Some((
                p.min_fret.unwrap_or(0),
                p.max_fret_limit.unwrap_or(max_fret).min(max_fret),
            ));
        }
        let optimized = optimize_steps(&steps, &tuning, max_fret, &model).map_err(|bad| {
            ErrorData::invalid_params(
                format!(
                    "{} moment(s) not playable within the given constraints (tuning/fret range)",
                    bad.len()
                ),
                None,
            )
        })?;
        let old_cost = tabmcp_theory::fingering::path_cost_with(&old_flat, &model);

        // Write back: walk beats in the same order; positions within a beat
        // are assigned to its notes sorted by pitch.
        let mut cursor = 0usize;
        let mut changed = 0usize;
        for measure in &mut measures {
            for beat in &mut measure.beats {
                let mut notes: Vec<&mut tabmcp_model::Note> = beat
                    .voices
                    .iter_mut()
                    .flat_map(|v| v.notes.iter_mut())
                    .filter(|n| !n.tied)
                    .collect();
                if notes.is_empty() {
                    continue;
                }
                notes.sort_by_key(|n| {
                    open_by_string
                        .get(&n.string)
                        .map(|&o| o.saturating_add(n.fret as u8))
                        .unwrap_or(0)
                });
                if let Some(set) = optimized.path.get(cursor) {
                    for (note, position) in notes.iter_mut().zip(set.iter()) {
                        if note.string != position.string_number || note.fret != position.fret {
                            changed += 1;
                        }
                        note.string = position.string_number;
                        note.fret = position.fret;
                    }
                }
                cursor += 1;
            }
        }

        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would re-finger {changed} note(s) across \
                     {} moment(s) ({chord_count} chord(s) re-voiced) in measures {from}-{to} of \
                     track {track_number}; hand-effort {old_cost:.1} -> {:.1}. All pitches stay \
                     identical. To apply, call again with confirm=true and expected_revision={}.",
                    steps.len(),
                    optimized.cost,
                    range.revision,
                ),
                revision: range.revision,
                measures_added: None,
                notes_before: Some(old_flat.len() as u32),
                notes_after: Some(old_flat.len() as u32),
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
                "Applied: re-fingered {changed} note(s) ({chord_count} chord(s) re-voiced) in \
                 measures {from}-{to} of track {track_number}; hand-effort {old_cost:.1} -> \
                 {:.1}; pitches unchanged. The user can undo with Cmd+Z.",
                optimized.cost,
            ),
            revision: result.new_revision,
            measures_added: None,
            notes_before: Some(old_flat.len() as u32),
            notes_after: Some(old_flat.len() as u32),
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
        description = "Generate a drum part in a groove style — 'rock' (default, kicks follow the guitar's accents), 'metal-gallop' (sixteenth kick gallop + ride), 'punk' (driving eighth kicks, open hats), 'halftime', 'blast' (alternating kick/snare 16ths), 'd-beat'. Written to a NEW percussion track (or target_track). Defaults to the selection. TWO-STEP: preview first, then confirm=true with expected_revision (undoable).",
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
        description = "Generate a COUNTERLINE: an answering melody written into the source riff's gaps - contrary motion, consonant on strong beats, an octave above. Turns one line into two interlocking lines (the djent/prog device). Fails when the source has no gaps: thin the riff first. Written to a NEW track (or target_track). TWO-STEP: preview, then confirm=true with expected_revision (undoable).",
        annotations(
            title = "Generate counterline",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_generate_counterline(
        &self,
        params: Parameters<GenerateParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        self.generate(params.0, GenerateKind::Counterline).await
    }

    #[tool(
        description = "Generate INTERLOCKED drums derived from the riff itself (not a template): kick lands in unison with the source's accents, snare holds the backbeat in any meter, hats keep the 8th grid, crash marks the entry. This is how breakdowns get their weight. Written to a NEW percussion track (or target_track). TWO-STEP: preview, then confirm=true with expected_revision (undoable).",
        annotations(
            title = "Generate interlocked drums",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_generate_interlock(
        &self,
        params: Parameters<GenerateParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        self.generate(params.0, GenerateKind::InterlockDrums).await
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
        description = "Toggle TuxGuitar's metronome click on/off for practice (state persists across plays).",
        annotations(title = "Metronome", read_only_hint = false, destructive_hint = false)
    )]
    async fn tuxguitar_toggle_metronome(&self) -> Result<String, ErrorData> {
        self.call_bridge(|client| client.toggle("action.transport.metronome"))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok("Metronome toggled.".into())
    }

    #[tool(
        description = "Toggle TuxGuitar's count-in (count-down before playback starts) on/off — useful when practicing over the loop.",
        annotations(title = "Count-in", read_only_hint = false, destructive_hint = false)
    )]
    async fn tuxguitar_toggle_count_in(&self) -> Result<String, ErrorData> {
        self.call_bridge(|client| client.toggle("action.transport.count-down"))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok("Count-in toggled.".into())
    }

    #[tool(
        description = "Jump playback to a specific measure and start playing from there (moves the caret too). Use tuxguitar_stop to stop.",
        annotations(
            title = "Play from measure",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_play_from(
        &self,
        params: Parameters<PlayFromParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        self.call_bridge(move |client| client.play_from(p.measure))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(format!("Playing from measure {}.", p.measure))
    }

    #[tool(
        description = "Detect and name the chords in a passage (power chords, triads, sevenths — e.g. E5, Am, G7) beat by beat, with a per-measure progression summary. Defaults to the user's active selection.",
        annotations(title = "Detect chords", read_only_hint = true)
    )]
    async fn tuxguitar_detect_chords(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<String, ErrorData> {
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
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track_number} does not exist"), None)
            })?;
        let open_by_string: std::collections::HashMap<u32, u8> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();
        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;

        let mut out = format!("Chords on track {track_number}, measures {from}-{to}:\n");
        let mut progression: Vec<String> = Vec::new();
        for measure in &range.measures {
            let mut names: Vec<String> = Vec::new();
            for beat in &measure.beats {
                let pcs: Vec<u8> = beat
                    .voices
                    .iter()
                    .flat_map(|v| &v.notes)
                    .filter(|n| !n.tied)
                    .filter_map(|n| open_by_string.get(&n.string).map(|&o| o + n.fret as u8))
                    .collect();
                if pcs.len() >= 2 {
                    if let Some(name) = tabmcp_theory::analysis::chord_name(&pcs) {
                        if names.last() != Some(&name) {
                            names.push(name.clone());
                        }
                        if progression.last() != Some(&name) {
                            progression.push(name);
                        }
                    }
                }
            }
            out.push_str(&format!(
                "  m{}: {}\n",
                measure.number,
                if names.is_empty() {
                    "-".into()
                } else {
                    names.join(" ")
                }
            ));
        }
        out.push_str(&format!(
            "Progression: {}\n",
            if progression.is_empty() {
                "(no chords detected)".into()
            } else {
                progression.join(" - ")
            }
        ));
        Ok(out)
    }

    #[tool(
        description = "Humanize a passage: vary note velocities by a deterministic +/- amount (default 8) so playback and MIDI export feel less robotic. Pitches and rhythm unchanged. TWO-STEP SAFETY: preview, then confirm=true with expected_revision. Undoable.",
        annotations(
            title = "Humanize velocities",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn tuxguitar_humanize(
        &self,
        params: Parameters<HumanizeParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        let amount = p.amount.unwrap_or(8).min(30) as i64;
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
        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let mut measures = range.measures;
        let mut changed = 0u32;
        for measure in &mut measures {
            for beat in &mut measure.beats {
                for voice in &mut beat.voices {
                    for note in &mut voice.notes {
                        // Deterministic jitter: same input -> same result.
                        let hash = (measure.number as u64).wrapping_mul(73856093)
                            ^ beat.start_tick.wrapping_mul(19349663)
                            ^ (note.string as u64).wrapping_mul(83492791);
                        let delta = (hash % (2 * amount as u64 + 1)) as i64 - amount;
                        let new_velocity = (note.velocity as i64 + delta).clamp(25, 120) as u32;
                        if new_velocity != note.velocity {
                            changed += 1;
                        }
                        note.velocity = new_velocity;
                    }
                }
            }
        }
        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would vary velocity on {changed} note(s) \
                     (+/-{amount}) in measures {from}-{to} of track {track_number}. To apply, \
                     call again with confirm=true and expected_revision={}.",
                    range.revision,
                ),
                revision: range.revision,
                measures_added: None,
                notes_before: Some(changed),
                notes_after: Some(changed),
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
                "Applied: humanized {changed} note velocities (+/-{amount}) in measures \
                 {from}-{to} of track {track_number}. The user can undo with Cmd+Z.",
            ),
            revision: result.new_revision,
            measures_added: None,
            notes_before: Some(changed),
            notes_after: Some(changed),
        }))
    }

    #[tool(
        description = "Import a MIDI file as playable tablature: reads ~/.tuxguitar-mcp/import.mid (put the file there first — e.g. exported from Logic), quantizes onto the grid, assigns strings/frets with the fingering optimizer (chords re-voiced), and writes to a NEW track. Assumes 4/4; channel-10 drums are skipped; max 32 measures. TWO-STEP: preview, then confirm=true with expected_revision. Follow up with the AI-Ear loop to clean the draft.",
        annotations(
            title = "Import MIDI",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_import_midi(
        &self,
        params: Parameters<ImportMidiParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        use tabmcp_theory::fingering::{optimize_steps, CostModel, Step};
        let Parameters(p) = params;
        let grid = p.quantize.unwrap_or(16).clamp(4, 32);
        let home = std::env::var("HOME").unwrap_or_default();
        let midi_path = PathBuf::from(&home).join(".tuxguitar-mcp/import.mid");
        let mut song_data = crate::import::parse_midi(&midi_path, grid, p.midi_track)
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        let mut truncated = 0usize;
        if song_data.measure_count > MAX_MEASURES_PER_READ as usize {
            truncated = song_data.measure_count - MAX_MEASURES_PER_READ as usize;
            song_data
                .steps
                .retain(|s| s.measure_index < MAX_MEASURES_PER_READ as usize);
            song_data.note_count = song_data.steps.iter().map(|s| s.pitches.len()).sum();
            song_data.measure_count = MAX_MEASURES_PER_READ as usize;
        }

        let strings = resolve_tuning(
            &None,
            &Some(
                p.preset
                    .clone()
                    .unwrap_or_else(|| "6-string standard".into()),
            ),
        )?;
        let tuning: Vec<(u32, u8)> = strings.iter().map(|s| (s.number, s.open_pitch)).collect();
        let steps: Vec<Step> = song_data
            .steps
            .iter()
            .map(|s| {
                if s.pitches.len() == 1 {
                    Step::Mono(s.pitches[0])
                } else {
                    Step::Chord(s.pitches.clone())
                }
            })
            .collect();
        let optimized =
            optimize_steps(&steps, &tuning, 24, &CostModel::default()).map_err(|bad| {
                ErrorData::invalid_params(
                    format!(
                        "{} imported moment(s) not playable on this tuning — try another preset",
                        bad.len()
                    ),
                    None,
                )
            })?;
        let measures = crate::import::build_measures(&song_data, &optimized.path);
        let track_name = p
            .track_name
            .clone()
            .unwrap_or_else(|| "Imported (AI)".into());

        let song = self.fetch_song().await?;
        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would import {} notes across {} measure(s) \
                     (MIDI track {} of {:?} available as (track, notes)) \
                     from import.mid into a new track \"{track_name}\" ({} tuning, {}th-note \
                     grid), fingered by the optimizer (effort {:.1}).{} To apply, call again \
                     with confirm=true and expected_revision={}.",
                    song_data.note_count,
                    song_data.measure_count,
                    song_data.chosen_track,
                    song_data.available_tracks,
                    tuning_names(&strings),
                    grid,
                    optimized.cost,
                    if truncated > 0 {
                        format!(" NOTE: {truncated} trailing measure(s) beyond the 32-measure cap were dropped.")
                    } else {
                        String::new()
                    },
                    song.revision,
                ),
                revision: song.revision,
                measures_added: Some(song_data.measure_count as u32),
                notes_before: Some(0),
                notes_after: Some(song_data.note_count as u32),
            }));
        }
        let expected_revision = p.expected_revision.ok_or_else(|| {
            ErrorData::invalid_params(
                "confirm=true requires expected_revision (from the preview call)",
                None,
            )
        })?;
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
        let name_for_create = track_name.clone();
        let created = self
            .call_bridge(move |client| client.create_track(&name_for_create, &strings, None, false))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let new_track = created
            .get("trackNumber")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let post_create = created
            .get("newRevision")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let result = self
            .call_bridge(move |client| {
                client.apply_replace_measures(new_track, 1, &measures, post_create)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied: imported {} notes into track {new_track} \"{track_name}\" \
                 ({} measures). Run the AI-Ear loop to clean the draft. Undo takes two Cmd+Z \
                 steps.",
                result.notes_after, song_data.measure_count,
            ),
            revision: result.new_revision,
            measures_added: Some(song_data.measure_count as u32),
            notes_before: Some(0),
            notes_after: Some(result.notes_after),
        }))
    }

    #[tool(
        description = "Change the time signature from a measure onward (or just that measure with to_end=false) — e.g. 7/8 for a djent section. NOTE: TuxGuitar re-bars trailing content by TOTAL TICKS, so shrinking meters can leave extra measures at the end (delete_measures cleans up). Undoable. Note: generators derive their grids from real measure lengths, so odd meters work downstream.",
        annotations(
            title = "Set time signature",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn tuxguitar_set_time_signature(
        &self,
        params: Parameters<SetTimeSignatureParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let to_end = p.to_end.unwrap_or(true);
        self.call_bridge(move |client| {
            client.set_time_signature(p.measure, p.numerator, p.denominator, to_end)
        })
        .await
        .map_err(BridgeCallError::into_error_data)?;
        Ok(format!(
            "Time signature set to {}/{} from measure {}{}.",
            p.numerator,
            p.denominator,
            p.measure,
            if to_end { " to the end" } else { " only" }
        ))
    }

    #[tool(
        description = "Change the key signature on a track from a measure onward (TuxGuitar key code: 0 = C/Am, positive = sharps 1..7, negative-as 8..14 = flats). Undoable.",
        annotations(
            title = "Set key signature",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_set_key_signature(
        &self,
        params: Parameters<SetKeySignatureParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let to_end = p.to_end.unwrap_or(true);
        let track = p.track_number.unwrap_or(1);
        self.call_bridge(move |client| client.set_key_signature(track, p.measure, p.key, to_end))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(format!(
            "Key signature {} set from measure {} on track {track}.",
            p.key, p.measure
        ))
    }

    #[tool(
        description = "Insert empty measures before a position (all tracks — measures are song-wide in TuxGuitar). Undoable.",
        annotations(
            title = "Insert measures",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn tuxguitar_insert_measures(
        &self,
        params: Parameters<InsertMeasuresParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let count = p.count.unwrap_or(1);
        self.call_bridge(move |client| client.insert_measures(p.at, count))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(format!(
            "Inserted {count} empty measure(s) before measure {}.",
            p.at
        ))
    }

    #[tool(
        description = "Delete measures from the song (all tracks — measures are song-wide). DESTRUCTIVE but undoable (one Cmd+Z per deleted measure).",
        annotations(
            title = "Delete measures",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn tuxguitar_delete_measures(
        &self,
        params: Parameters<InsertMeasuresParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let count = p.count.unwrap_or(1);
        self.call_bridge(move |client| client.delete_measures(p.at, count))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(format!(
            "Deleted {count} measure(s) starting at measure {}.",
            p.at
        ))
    }

    #[tool(
        description = "Copy a measure range to another place (same or different track) - song forms without resending content. Destination content is replaced (appended past the end). TWO-STEP: preview, then confirm=true with expected_revision. Undoable.",
        annotations(
            title = "Copy measures",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn tuxguitar_copy_measures(
        &self,
        params: Parameters<CopyMeasuresParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        if p.from_measure == 0
            || p.to_measure < p.from_measure
            || p.to_measure - p.from_measure + 1 > MAX_MEASURES_PER_READ
        {
            return Err(ErrorData::invalid_params(
                format!("invalid source range (max {MAX_MEASURES_PER_READ} measures)"),
                None,
            ));
        }
        let source_track = p.source_track;
        let (from, to) = (p.from_measure, p.to_measure);
        let range = self
            .call_bridge(move |client| client.read_measures(source_track, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let mut measures = range.measures;
        for (i, measure) in measures.iter_mut().enumerate() {
            measure.number = p.dest_measure + i as u32;
        }
        let count = measures.len() as u32;
        let dest_track = p.dest_track.unwrap_or(p.source_track);
        let notes = count_notes(&measures);

        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY - nothing changed. Would copy measures {from}-{to} of track \
                     {source_track} ({notes} notes) to measures {}-{} of track {dest_track}. \
                     To apply, call again with confirm=true and expected_revision={}.",
                    p.dest_measure,
                    p.dest_measure + count - 1,
                    range.revision,
                ),
                revision: range.revision,
                measures_added: None,
                notes_before: None,
                notes_after: Some(notes),
            }));
        }
        let expected_revision = p.expected_revision.ok_or_else(|| {
            ErrorData::invalid_params(
                "confirm=true requires expected_revision (from the preview call)",
                None,
            )
        })?;
        let dest_from = p.dest_measure;
        let result = self
            .call_bridge(move |client| {
                client.apply_replace_measures(dest_track, dest_from, &measures, expected_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied: copied {count} measure(s) to track {dest_track} at measure \
                 {dest_from}. The user can undo with Cmd+Z.",
            ),
            revision: result.new_revision,
            measures_added: Some(result.measures_added),
            notes_before: Some(result.notes_before),
            notes_after: Some(result.notes_after),
        }))
    }

    #[tool(
        description = "Composition recipes per style: a consistent rubric of scales, tuning presets, tempo, meters, rhythmic cells, techniques, drums, signature devices, song-section arc, mood, difficulty, an AVOID list, and numeric evaluation targets. Call without a style to list the catalog plus universal instrument roles. GENRE CROSSOVER: pass a blend like '60% death metal, 40% doom' (or 'djent + doom') to merge stylistic characteristics by weight - traits, never copied riffs. USE THIS before composing in a named genre.",
        annotations(title = "Style guide", read_only_hint = true)
    )]
    async fn tuxguitar_style_guide(
        &self,
        params: Parameters<StyleGuideParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        match p.style.as_deref() {
            Some(spec) if spec.contains('%') || spec.contains('+') || spec.contains(',') => {
                tabmcp_theory::styles::parse_blend(spec)
                    .map(|parts| tabmcp_theory::styles::describe_blend(&parts))
                    .ok_or_else(|| {
                        ErrorData::invalid_params(
                            format!(
                                "could not parse blend '{spec}'; expected e.g. \"60% death \
                                 metal, 40% doom\" with known styles: {}",
                                tabmcp_theory::styles::STYLES
                                    .iter()
                                    .map(|s| s.name)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            None,
                        )
                    })
            }
            Some(name) => tabmcp_theory::styles::style_guide(name)
                .map(tabmcp_theory::styles::describe)
                .ok_or_else(|| {
                    ErrorData::invalid_params(
                        format!(
                            "unknown style '{name}'; available: {}",
                            tabmcp_theory::styles::STYLES
                                .iter()
                                .map(|s| s.name)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        None,
                    )
                }),
            None => Ok(format!(
                "Available styles (pass one for the full recipe, or a blend like \
                 '60% death metal, 40% doom'):\n{}\n\n{}",
                tabmcp_theory::styles::STYLES
                    .iter()
                    .map(|s| format!("  {} — {} BPM, {}", s.name, s.tempo, s.drums))
                    .collect::<Vec<_>>()
                    .join("\n"),
                tabmcp_theory::styles::ROLES,
            )),
        }
    }

    #[tool(
        description = "Vary a riff mechanically — 9 transforms: 'displace' shifts onsets by N ticks (wrapping per measure; odd ticks = polyrhythmic); 'retrograde' reverses pitch order; 'invert' mirrors pitches around the first note; 'octave' shifts by whole octaves (octaves param, negative = down); 'augment' doubles durations (result is 2x measures); 'diminish' halves durations (material compresses into the first half); 'pedal' fills empty grid slots with palm-muted pedal tones (pedal_string/pedal_fret, grid = amount); 'regroup' accents a grouping like 3+3+2 cycling across barlines (implied polymeter); 'swap_dynamics' flips palm-mutes and accents (call-and-response variant). Meter-aware. Write in place or to dest_measure. TWO-STEP: preview, then confirm=true with expected_revision. Undoable.",
        annotations(title = "Vary riff", read_only_hint = false, destructive_hint = true)
    )]
    async fn tuxguitar_vary_riff(
        &self,
        params: Parameters<VaryRiffParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        let track = p.track_number;
        let (from, to) = (p.from_measure, p.to_measure);
        if from == 0 || to < from || to - from + 1 > MAX_MEASURES_PER_READ {
            return Err(ErrorData::invalid_params("invalid range", None));
        }
        let range = self
            .call_bridge(move |client| client.read_measures(track, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let mut measures = range.measures;
        use tabmcp_theory::transforms;
        let needs_tuning = matches!(p.transform.as_str(), "invert" | "octave" | "pedal");
        let tuning: Vec<(u32, u8)> = if needs_tuning {
            let song = self.fetch_song().await?;
            let strings = song
                .tracks
                .iter()
                .find(|t| t.number == track)
                .map(|t| {
                    t.strings
                        .iter()
                        .map(|s| (s.number, s.open_pitch))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if strings.is_empty() {
                return Err(ErrorData::invalid_params(
                    format!("track {track} not found"),
                    None,
                ));
            }
            strings
        } else {
            Vec::new()
        };
        match p.transform.as_str() {
            "displace" => transforms::displace(&mut measures, p.amount.unwrap_or(480) as u64),
            "retrograde" => transforms::retrograde(&mut measures),
            "invert" => transforms::invert(&mut measures, &tuning),
            "octave" => transforms::octave_shift(&mut measures, &tuning, p.octaves.unwrap_or(1)),
            "augment" => measures = transforms::augment(&measures),
            "diminish" => transforms::diminish(&mut measures),
            "pedal" => {
                // Default pedal tone: the lowest string, open.
                let low = tuning
                    .iter()
                    .min_by_key(|&&(_, open)| open)
                    .map(|&(s, _)| s)
                    .unwrap_or(6);
                transforms::pedal_fill(
                    &mut measures,
                    p.pedal_string.unwrap_or(low),
                    p.pedal_fret.unwrap_or(0),
                    p.amount.unwrap_or(240) as u64,
                );
            }
            "regroup" => {
                let spec = p.grouping.as_deref().unwrap_or("3+3+2");
                let groups: Vec<u32> = spec
                    .split(['+', ',', ' '])
                    .filter(|s| !s.is_empty())
                    .map(|s| s.trim().parse::<u32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| {
                        ErrorData::invalid_params(
                            format!("bad grouping '{spec}'; expected e.g. \"3+3+2\""),
                            None,
                        )
                    })?;
                transforms::accent_group(&mut measures, &groups, p.amount.unwrap_or(240) as u64);
            }
            "swap_dynamics" => transforms::swap_dynamics(&mut measures),
            other => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "unknown transform '{other}'; available: displace, retrograde, invert, \
                         octave, augment, diminish, pedal, regroup, swap_dynamics"
                    ),
                    None,
                ))
            }
        }
        let dest = p.dest_measure.unwrap_or(from);
        for (i, measure) in measures.iter_mut().enumerate() {
            measure.number = dest + i as u32;
        }
        let notes = count_notes(&measures);
        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY - nothing changed. Would apply '{}' to measures {from}-{to} \
                     of track {track} and write {notes} notes at measure {dest}. To apply, \
                     call again with confirm=true and expected_revision={}.",
                    p.transform, range.revision,
                ),
                revision: range.revision,
                measures_added: None,
                notes_before: Some(notes),
                notes_after: Some(notes),
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
                client.apply_replace_measures(track, dest, &measures, expected_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied '{}' to measures {from}-{to} of track {track}, written at measure \
                 {dest}. The user can undo with Cmd+Z.",
                p.transform,
            ),
            revision: result.new_revision,
            measures_added: Some(result.measures_added),
            notes_before: Some(notes),
            notes_after: Some(result.notes_after),
        }))
    }

    #[tool(
        description = "Rate how hard a passage is for a human player: 1-10 with itemized reasons (notes/sec, chord stretches, fast position shifts, string skips, 16th-run endurance, technique load). Defaults to the selection, falls back to track 1 whole song. Use it to keep parts playable — or to build practice material at a target level; pair with tuxguitar_optimize_fingering to bring a hard passage down.",
        annotations(title = "Analyze difficulty", read_only_hint = true)
    )]
    async fn tuxguitar_analyze_difficulty(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<String, ErrorData> {
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
        let last_measure = song.headers.len() as u32;
        let (from, to) = match (p.from_measure, p.to_measure) {
            (Some(from), Some(to)) => (from, to),
            (Some(from), None) => (from, from),
            (None, _) if selection.active => (
                selection.from_measure.unwrap_or(1),
                selection.to_measure.unwrap_or(last_measure),
            ),
            _ => (1, last_measure.max(1)),
        };
        if from == 0 || to < from || to > last_measure || to - from + 1 > MAX_MEASURES_PER_READ {
            return Err(ErrorData::invalid_params(
                format!("invalid measure range {from}-{to} (score has 1-{last_measure}, max {MAX_MEASURES_PER_READ} per call)"),
                None,
            ));
        }
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track_number} not found"), None)
            })?;
        let tuning: Vec<(u32, u8)> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();
        let tempo = song
            .headers
            .iter()
            .find(|h| h.number == from)
            .map(|h| h.tempo_bpm)
            .unwrap_or(120);
        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let report = tabmcp_theory::difficulty::analyze(&range.measures, &tuning, tempo);
        Ok(tabmcp_theory::difficulty::describe(
            &report,
            &format!(
                "Track {track_number} \"{}\", measures {from}-{to} at {tempo} BPM",
                track.name
            ),
        ))
    }

    #[tool(
        description = "RIFF EVOLUTION — hill-climb a riff through N generations: each generation tries every vary_riff mutation (displace, invert, octave, diminish, pedal, regroup, swap_dynamics), scores each candidate with the AI Ear fitness (groove + motif + dynamics + syncopation + breathing room), and keeps the best only if it improves. Returns the lineage ('gen 2: regroup — 61% -> 68%'). Deterministic. TWO-STEP: preview shows the lineage, confirm=true with expected_revision writes the winner. Undoable. For pitch-level evolution (new notes in the same DNA), use tuxguitar_riff_dna and write the material yourself.",
        annotations(title = "Evolve riff", read_only_hint = false, destructive_hint = true)
    )]
    async fn tuxguitar_evolve_riff(
        &self,
        params: Parameters<EvolveRiffParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        let track = p.track_number;
        let (from, to) = (p.from_measure, p.to_measure);
        if from == 0 || to < from || to - from + 1 > MAX_MEASURES_PER_READ {
            return Err(ErrorData::invalid_params("invalid range", None));
        }
        let generations = p.generations.unwrap_or(4).clamp(1, 8);
        let song = self.fetch_song().await?;
        let tuning: Vec<(u32, u8)> = song
            .tracks
            .iter()
            .find(|t| t.number == track)
            .map(|t| {
                t.strings
                    .iter()
                    .map(|s| (s.number, s.open_pitch))
                    .collect()
            })
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track} not found"), None)
            })?;
        let range = self
            .call_bridge(move |client| client.read_measures(track, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;

        fn fitness(report: &tabmcp_theory::critique::CritiqueReport) -> f64 {
            let window = |s: f64, lo: f64, hi: f64| {
                if s >= lo && s <= hi {
                    1.0
                } else {
                    let dist = if s < lo { lo - s } else { s - hi };
                    (1.0 - dist * 4.0).max(0.0)
                }
            };
            0.30 * report.groove_consistency
                + 0.20 * (report.motif_repetition.min(0.5) / 0.5)
                + 0.15 * (report.velocity_std / 12.0).min(1.0)
                + 0.20 * window(report.syncopation, 0.15, 0.60)
                + 0.15 * (report.rest_share / 0.25).min(1.0)
        }

        use tabmcp_theory::transforms;
        let low_string = tuning
            .iter()
            .min_by_key(|&&(_, open)| open)
            .map(|&(s, _)| s)
            .unwrap_or(6);
        type Mutation = (
            &'static str,
            Box<dyn Fn(&mut Vec<tabmcp_model::Measure>) + Send + Sync>,
        );
        let t1 = tuning.clone();
        let t2 = tuning.clone();
        let t3 = tuning.clone();
        let mutations: Vec<Mutation> = vec![
            ("displace 8th", Box::new(|m| transforms::displace(m, 480))),
            ("displace 16th", Box::new(|m| transforms::displace(m, 240))),
            ("invert", Box::new(move |m| transforms::invert(m, &t1))),
            ("octave up", Box::new(move |m| transforms::octave_shift(m, &t2, 1))),
            ("diminish", Box::new(|m| transforms::diminish(m))),
            (
                "pedal fill",
                Box::new(move |m| {
                    let _ = &t3;
                    transforms::pedal_fill(m, low_string, 0, 240)
                }),
            ),
            (
                "regroup 3+3+2",
                Box::new(|m| transforms::accent_group(m, &[3, 3, 2], 240)),
            ),
            ("swap dynamics", Box::new(|m| transforms::swap_dynamics(m))),
        ];

        let mut current = range.measures.clone();
        let mut current_fit = fitness(&tabmcp_theory::critique::critique(&current, &tuning));
        let seed_fit = current_fit;
        let mut lineage: Vec<String> = Vec::new();
        for generation in 1..=generations {
            let mut best: Option<(&str, Vec<tabmcp_model::Measure>, f64)> = None;
            for (label, mutate) in &mutations {
                let mut candidate = current.clone();
                mutate(&mut candidate);
                let fit = fitness(&tabmcp_theory::critique::critique(&candidate, &tuning));
                if best.as_ref().map(|(_, _, f)| fit > *f).unwrap_or(true) {
                    best = Some((label, candidate, fit));
                }
            }
            match best {
                Some((label, candidate, fit)) if fit > current_fit + 0.005 => {
                    lineage.push(format!(
                        "gen {generation}: {label} — fitness {:.0}% -> {:.0}%",
                        current_fit * 100.0,
                        fit * 100.0
                    ));
                    current = candidate;
                    current_fit = fit;
                }
                _ => {
                    lineage.push(format!(
                        "gen {generation}: no mutation improves — evolution stable at {:.0}%",
                        current_fit * 100.0
                    ));
                    break;
                }
            }
        }

        let dest = p.dest_measure.unwrap_or(from);
        for (i, measure) in current.iter_mut().enumerate() {
            measure.number = dest + i as u32;
        }
        let notes = count_notes(&current);
        let lineage_text = lineage.join("; ");
        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY - nothing changed. Evolution of track {track} measures \
                     {from}-{to}: {lineage_text}. Final fitness {:.0}% (seed was {:.0}%). \
                     Would write {notes} notes at measure {dest}. To apply, call again with \
                     confirm=true and expected_revision={}.",
                    current_fit * 100.0,
                    seed_fit * 100.0,
                    range.revision,
                ),
                revision: range.revision,
                measures_added: None,
                notes_before: Some(notes),
                notes_after: Some(notes),
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
                client.apply_replace_measures(track, dest, &current, expected_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied evolution to track {track}: {lineage_text}. Final fitness {:.0}%. \
                 Written at measure {dest}. The user can undo with Cmd+Z.",
                current_fit * 100.0,
            ),
            revision: result.new_revision,
            measures_added: Some(result.measures_added),
            notes_before: Some(notes),
            notes_after: Some(result.notes_after),
        }))
    }

    #[tool(
        description = "CONSTRAINT-GUIDED RIFF GENERATION — beam search over the rhythm-cell alphabet and a scale's pitch space, scored WHILE generating: accent offsets get roots and accent marks (set them to the kick pattern for instant unison), syncopation stays in the target window, motion is mostly stepwise with b2/tritone spice, an AABA' form gives the riff grammar, velocities and palm mutes come pre-shaped. Deterministic: same constraints, same riff. Rhythm cells: quarter, 8ths, 16ths, gallop, reverse-gallop, herta, offbeat-8ths, and-of-one, tresillo, hemiola, triplet-8ths, quintuplet, dotted-8ths, sixteenth-rest-start, rest-8th, rest-quarter. Destination measures' meters define the bars (works in 7/8 etc.). TWO-STEP: preview, then confirm=true with expected_revision. Undoable.",
        annotations(title = "Generate riff", read_only_hint = false, destructive_hint = true)
    )]
    async fn tuxguitar_generate_riff(
        &self,
        params: Parameters<GenerateRiffParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        let track_number = p.track_number;
        let (from, to) = (p.from_measure, p.to_measure);
        if from == 0 || to < from || to - from + 1 > MAX_MEASURES_PER_READ {
            return Err(ErrorData::invalid_params("invalid range", None));
        }
        let song = self.fetch_song().await?;
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track_number} not found"), None)
            })?;
        let tuning: Vec<(u32, u8)> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();
        let max_fret = if track.max_fret > 0 { track.max_fret } else { 24 };

        // Scale: "A phrygian dominant" -> root pc + interval steps.
        let (root_name, scale_name) = p
            .scale
            .trim()
            .split_once(' ')
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    "scale must be '<root> <name>', e.g. 'A phrygian dominant'",
                    None,
                )
            })?;
        let root_pc = tabmcp_theory::parse_note(&format!("{root_name}4"))
            .map(|p| p % 12)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("unknown root note '{root_name}'"), None)
            })?;
        let steps = tabmcp_theory::analysis::SCALES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(scale_name.trim()))
            .map(|(_, steps)| *steps)
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    format!(
                        "unknown scale '{}'; catalog has {} scales - try 'phrygian dominant', \
                         'harmonic minor', 'hirajoshi', ...",
                        scale_name.trim(),
                        tabmcp_theory::analysis::SCALES.len()
                    ),
                    None,
                )
            })?;

        // Register: default low open string .. +19 semitones.
        let lowest_open = tuning.iter().map(|&(_, o)| o).min().unwrap_or(40);
        let parse_reg = |spec: &Option<String>, default: u8| -> Result<u8, ErrorData> {
            match spec {
                Some(s) => tabmcp_theory::parse_note(s).ok_or_else(|| {
                    ErrorData::invalid_params(format!("bad note name '{s}'"), None)
                }),
                None => Ok(default),
            }
        };
        let reg_lo = parse_reg(&p.register_low, lowest_open)?;
        let reg_hi = parse_reg(&p.register_high, lowest_open.saturating_add(19))?;
        let pitch_pool: Vec<u8> = (reg_lo..=reg_hi)
            .filter(|pitch| steps.contains(&((pitch + 12 - root_pc) % 12)))
            .collect();

        // Cells, accents, windows.
        let cells: Vec<&'static tabmcp_theory::cells::RhythmCell> = match &p.cells {
            Some(spec) => spec
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|name| {
                    tabmcp_theory::cells::cell(name).ok_or_else(|| {
                        ErrorData::invalid_params(
                            format!(
                                "unknown cell '{}'; catalog:\n{}",
                                name.trim(),
                                tabmcp_theory::cells::catalog()
                            ),
                            None,
                        )
                    })
                })
                .collect::<Result<_, _>>()?,
            None => Vec::new(),
        };
        let parse_range_f = |spec: &Option<String>, default: (f64, f64)| {
            spec.as_ref()
                .and_then(|s| s.split_once('-'))
                .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
                .unwrap_or(default)
        };
        let parse_range_u = |spec: &Option<String>, default: (usize, usize)| {
            spec.as_ref()
                .and_then(|s| s.split_once('-'))
                .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
                .unwrap_or(default)
        };
        let accents: Vec<u64> = p
            .accents
            .as_deref()
            .unwrap_or("0")
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        // Destination meters -> measure lengths.
        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let mut measure_lens: Vec<u64> = Vec::new();
        for i in 0..range.measures.len() {
            let len = if i + 1 < range.measures.len() {
                range.measures[i + 1]
                    .start_tick
                    .saturating_sub(range.measures[i].start_tick)
            } else {
                measure_lens.last().copied().unwrap_or(3840)
            };
            measure_lens.push(if len > 0 { len } else { 3840 });
        }

        let constraints = tabmcp_theory::search::RiffConstraints {
            pitch_pool,
            root_pc,
            cells,
            measure_lens,
            accents,
            syncopation: parse_range_f(&p.syncopation, (0.15, 0.55)),
            notes_per_measure: parse_range_u(&p.density, (4, 10)),
            max_pitch_repeat: 3,
            palm_mute_low: p.palm_mute_low.unwrap_or(true),
        };
        let first_start = range
            .measures
            .first()
            .map(|m| m.start_tick)
            .unwrap_or(960);
        let (mut measures, explanation) = tabmcp_theory::search::generate_riff(
            &constraints,
            &tuning,
            max_fret,
            from,
            first_start,
        )
        .map_err(|e| ErrorData::invalid_params(e, None))?;
        for (i, measure) in measures.iter_mut().enumerate() {
            measure.number = from + i as u32;
        }
        let notes = count_notes(&measures);
        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY - nothing changed. Generated riff for track {track_number} \
                     measures {from}-{to} in {} - {explanation}. Would write {notes} notes. \
                     To apply, call again with confirm=true and expected_revision={}.",
                    p.scale, range.revision,
                ),
                revision: range.revision,
                measures_added: None,
                notes_before: Some(notes),
                notes_after: Some(notes),
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
                "Applied generated riff to track {track_number} measures {from}-{to}: \
                 {explanation}. The user can undo with Cmd+Z.",
            ),
            revision: result.new_revision,
            measures_added: Some(result.measures_added),
            notes_before: Some(notes),
            notes_after: Some(result.notes_after),
        }))
    }

    #[tool(
        description = "RE-BAR — pour a passage, as one continuous stream, into measures with DIFFERENT time signatures: the notes keep their flow positions, only the barlines move (the signature djent device - the same riff re-barred across 7/8, 5/4, 4/4). Workflow: set the destination measures' meters first (tuxguitar_set_time_signature), then rebar the source into them. Notes whose duration crosses a new barline ring across it. TWO-STEP: preview, then confirm=true with expected_revision. Undoable.",
        annotations(title = "Re-bar riff", read_only_hint = false, destructive_hint = true)
    )]
    async fn tuxguitar_rebar(
        &self,
        params: Parameters<RebarParams>,
    ) -> Result<Json<EditOutcome>, ErrorData> {
        let Parameters(p) = params;
        let track = p.track_number;
        if p.from_measure == 0
            || p.to_measure < p.from_measure
            || p.dest_from == 0
            || p.dest_to < p.dest_from
            || p.to_measure - p.from_measure + 1 > MAX_MEASURES_PER_READ
            || p.dest_to - p.dest_from + 1 > MAX_MEASURES_PER_READ
        {
            return Err(ErrorData::invalid_params("invalid range", None));
        }
        let (source_from, source_to) = (p.from_measure, p.to_measure);
        let source = self
            .call_bridge(move |client| client.read_measures(track, source_from, source_to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let (dest_from, dest_to) = (p.dest_from, p.dest_to);
        let dest = self
            .call_bridge(move |client| client.read_measures(track, dest_from, dest_to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let mut rebarred =
            tabmcp_theory::transforms::rebar(&source.measures, &dest.measures);
        for (i, measure) in rebarred.iter_mut().enumerate() {
            measure.number = dest_from + i as u32;
        }
        let notes = count_notes(&rebarred);
        let source_notes = count_notes(&source.measures);
        if !p.confirm {
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY - nothing changed. Would re-bar measures {source_from}-\
                     {source_to} into the meter structure of measures {dest_from}-{dest_to} \
                     ({notes} of {source_notes} source notes fit; overflow past the last \
                     destination barline is dropped). To apply, call again with confirm=true \
                     and expected_revision={}.",
                    dest.revision,
                ),
                revision: dest.revision,
                measures_added: None,
                notes_before: Some(source_notes),
                notes_after: Some(notes),
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
                client.apply_replace_measures(track, dest_from, &rebarred, expected_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Re-barred measures {source_from}-{source_to} into {dest_from}-{dest_to}. \
                 The user can undo with Cmd+Z.",
            ),
            revision: result.new_revision,
            measures_added: Some(result.measures_added),
            notes_before: Some(source_notes),
            notes_after: Some(result.notes_after),
        }))
    }

    #[tool(
        description = "THEME TRACKER (musical memory) — map how motifs travel through the song: which section introduces motif A, which sections restate, vary, invert, retrograde, fragment or extend it, and where call-and-response phrasing happens (paired measures with the same rhythm and answering contour). Flags a song that forgets its own material. Sections come from markers and meter changes. Classical development devices, automated.",
        annotations(title = "Track themes", read_only_hint = true)
    )]
    async fn tuxguitar_track_themes(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let song = self.fetch_song().await?;
        let last = song.headers.len() as u32;
        if last == 0 {
            return Err(ErrorData::invalid_params("empty score", None));
        }
        let track_number = p.track_number.unwrap_or(1);
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track_number} not found"), None)
            })?;
        let tuning: Vec<(u32, u8)> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();
        let (from, to) = (p.from_measure.unwrap_or(1), p.to_measure.unwrap_or(last));
        let measures = self.read_measures_chunked(track_number, from, to).await?;
        let boundaries = Self::section_boundaries(&song, from, to);
        let mut sections: Vec<(String, u32, u32, Vec<tabmcp_model::Measure>)> = Vec::new();
        for (i, (start, label)) in boundaries.iter().enumerate() {
            let end = boundaries.get(i + 1).map(|(m, _)| m - 1).unwrap_or(to);
            let slice: Vec<tabmcp_model::Measure> = measures
                .iter()
                .filter(|m| m.number >= *start && m.number <= end)
                .cloned()
                .collect();
            sections.push((label.clone(), *start, end, slice));
        }
        let report = tabmcp_theory::themes::analyze_themes(&sections, &tuning);
        Ok(format!(
            "Track {track_number} \"{}\":\n{}",
            track.name,
            tabmcp_theory::themes::describe(&report)
        ))
    }

    #[tool(
        description = "GUITAR REALISM CHECK — catch what a human hand cannot do before a player tries: impossible chord stretches (>6 frets), the same string sounded twice in one beat, frets past the neck, string-breaking bends (>4 semitones), stiff bends on wound low strings, and open strings ringing inside high positions. Checks every non-percussion track in the range (or one track if specified). Fix 'impossible' items; judge 'awkward' ones.",
        annotations(title = "Check realism", read_only_hint = true)
    )]
    async fn tuxguitar_check_realism(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let song = self.fetch_song().await?;
        let last = song.headers.len() as u32;
        let (from, to) = (p.from_measure.unwrap_or(1), p.to_measure.unwrap_or(last));
        let mut out = String::new();
        for track in &song.tracks {
            if track.is_percussion {
                continue;
            }
            if let Some(wanted) = p.track_number {
                if track.number != wanted {
                    continue;
                }
            }
            let measures = self.read_measures_chunked(track.number, from, to).await?;
            let report = tabmcp_theory::realism::check(&measures, track.max_fret.max(19));
            out.push_str(&tabmcp_theory::realism::describe(
                &report,
                &format!("Track {} \"{}\"", track.number, track.name),
            ));
        }
        if out.is_empty() {
            out = "No melodic tracks to check.".into();
        }
        Ok(out)
    }

    #[tool(
        description = "STYLE MATCH — 'what makes this sound like death metal?': measure the score's characteristics (tempo, syncopation, detected scale, technique mix) against every style rubric and rank the closest matches with per-criterion reasons. Extracts stylistic characteristics, never band riffs. Use it to verify a target style landed, or to discover what a piece actually is.",
        annotations(title = "Style match", read_only_hint = true)
    )]
    async fn tuxguitar_style_match(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let song = self.fetch_song().await?;
        let last = song.headers.len() as u32;
        let (from, to) = (p.from_measure.unwrap_or(1), p.to_measure.unwrap_or(last));
        let tempo = song
            .headers
            .iter()
            .find(|h| h.number == from)
            .map(|h| h.tempo_bpm)
            .unwrap_or(120);
        let mut syncopations = Vec::new();
        let mut palm_mute_share = 0.0f64;
        let mut all_events: Vec<NoteEvent> = Vec::new();
        let mut note_total = 0usize;
        let mut muted_total = 0usize;
        for track in &song.tracks {
            if track.is_percussion {
                continue;
            }
            let tuning: Vec<(u32, u8)> = track
                .strings
                .iter()
                .map(|s| (s.number, s.open_pitch))
                .collect();
            let open: std::collections::HashMap<u32, u8> = tuning.iter().copied().collect();
            let measures = self.read_measures_chunked(track.number, from, to).await?;
            for measure in &measures {
                for beat in &measure.beats {
                    for voice in &beat.voices {
                        for note in &voice.notes {
                            if note.tied {
                                continue;
                            }
                            note_total += 1;
                            if note.effects.palm_mute {
                                muted_total += 1;
                            }
                            if let Some(&o) = open.get(&note.string) {
                                all_events.push(NoteEvent {
                                    pitch: o.saturating_add(note.fret as u8),
                                    weight: 480,
                                });
                            }
                        }
                    }
                }
            }
            let report = tabmcp_theory::critique::critique(&measures, &tuning);
            if report.velocity_mean > 0.0 {
                syncopations.push(report.syncopation);
            }
        }
        if note_total > 0 {
            palm_mute_share = muted_total as f64 / note_total as f64;
        }
        let syncopation = if syncopations.is_empty() {
            0.0
        } else {
            syncopations.iter().sum::<f64>() / syncopations.len() as f64
        };
        let detected = detect_scales(&all_events);
        let scale_name = detected
            .first()
            .map(|s| s.scale.to_ascii_lowercase())
            .unwrap_or_default();

        let mut ranked: Vec<(f64, String)> = tabmcp_theory::styles::STYLES
            .iter()
            .map(|g| {
                let tempo_ok =
                    tempo >= g.tempo_range.0 as u32 && tempo <= g.tempo_range.1 as u32;
                let sync_ok = syncopation >= g.syncopation.0 && syncopation <= g.syncopation.1;
                let scale_ok = !scale_name.is_empty()
                    && g.scales.to_ascii_lowercase().contains(&scale_name);
                let technique_hint = g.techniques.to_ascii_lowercase();
                let mute_ok = if technique_hint.contains("palmmute")
                    || technique_hint.contains("chug")
                {
                    palm_mute_share > 0.15
                } else {
                    palm_mute_share < 0.5
                };
                let score = 0.30 * tempo_ok as u8 as f64
                    + 0.25 * sync_ok as u8 as f64
                    + 0.30 * scale_ok as u8 as f64
                    + 0.15 * mute_ok as u8 as f64;
                let detail = format!(
                    "{:>3.0}% {} - tempo {tempo} {} {}-{}; syncopation {:.0}% {} {:.0}-{:.0}%; \
                     scale {} {}; palm-mute {:.0}% {}",
                    score * 100.0,
                    g.name,
                    if tempo_ok { "fits" } else { "outside" },
                    g.tempo_range.0,
                    g.tempo_range.1,
                    syncopation * 100.0,
                    if sync_ok { "fits" } else { "outside" },
                    g.syncopation.0 * 100.0,
                    g.syncopation.1 * 100.0,
                    if scale_name.is_empty() { "unknown" } else { &scale_name },
                    if scale_ok { "matches" } else { "does not match" },
                    palm_mute_share * 100.0,
                    if mute_ok { "consistent" } else { "inconsistent" },
                );
                (score, detail)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        let top: Vec<String> = ranked.into_iter().take(3).map(|(_, d)| d).collect();
        Ok(format!(
            "STYLE MATCH (characteristics, not copied riffs), measures {from}-{to}:\n{}\n\
             These are measured traits - interval choices, rhythm, density. To lean \
             into a style, pull its recipe with tuxguitar_style_guide.",
            top.join("\n")
        ))
    }

    #[tool(
        description = "PRODUCER MODE — arrangement-level suggestions instead of note edits: where to double a riff, drop instruments out, harmonize, hand a section to the bass, or set up a unison. Data-driven from section densities, dynamics, and structure; every suggestion names the tool that executes it. Run it when the notes are right but the song feels flat.",
        annotations(title = "Producer notes", read_only_hint = true)
    )]
    async fn tuxguitar_producer_notes(
        &self,
        params: Parameters<AnalysisScopeParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let song = self.fetch_song().await?;
        let last = song.headers.len() as u32;
        let (from, to) = (p.from_measure.unwrap_or(1), p.to_measure.unwrap_or(last));
        let boundaries = Self::section_boundaries(&song, from, to);

        struct TrackData {
            number: u32,
            name: String,
            is_percussion: bool,
            mean_pitch: f64,
            measures: Vec<tabmcp_model::Measure>,
        }
        let mut tracks: Vec<TrackData> = Vec::new();
        for track in &song.tracks {
            let open: std::collections::HashMap<u32, u8> = track
                .strings
                .iter()
                .map(|s| (s.number, s.open_pitch))
                .collect();
            let measures = self.read_measures_chunked(track.number, from, to).await?;
            let mut pitch_sum = 0f64;
            let mut count = 0usize;
            for measure in &measures {
                for beat in &measure.beats {
                    for voice in &beat.voices {
                        for note in &voice.notes {
                            if let Some(&o) = open.get(&note.string) {
                                pitch_sum += (o as u32 + note.fret) as f64;
                                count += 1;
                            }
                        }
                    }
                }
            }
            tracks.push(TrackData {
                number: track.number,
                name: track.name.clone(),
                is_percussion: track.is_percussion,
                mean_pitch: if count == 0 { 0.0 } else { pitch_sum / count as f64 },
                measures,
            });
        }

        // Per-section energy: note count weighted by velocity.
        let section_ranges: Vec<(String, u32, u32)> = boundaries
            .iter()
            .enumerate()
            .map(|(i, (start, label))| {
                let end = boundaries.get(i + 1).map(|(m, _)| m - 1).unwrap_or(to);
                (label.clone(), *start, end)
            })
            .collect();
        let energy_of = |track: &TrackData, lo: u32, hi: u32| -> f64 {
            track
                .measures
                .iter()
                .filter(|m| m.number >= lo && m.number <= hi)
                .flat_map(|m| &m.beats)
                .flat_map(|b| &b.voices)
                .flat_map(|v| &v.notes)
                .map(|n| n.velocity as f64 / 127.0)
                .sum()
        };
        let mut notes: Vec<String> = Vec::new();

        let melodic: Vec<&TrackData> = tracks.iter().filter(|t| !t.is_percussion).collect();
        let guitars: Vec<&&TrackData> = melodic
            .iter()
            .filter(|t| t.mean_pitch >= 45.0) // above bass territory
            .collect();
        if guitars.len() == 1 {
            notes.push(format!(
                "Double the guitar: create a second track (tuxguitar_create_track) and \
                 copy_measures track {}'s riff onto it - width for free; pan hard \
                 left/right later in the DAW.",
                guitars[0].number
            ));
        }
        if let Some(peak) = section_ranges
            .iter()
            .max_by(|a, b| {
                let ea: f64 = tracks.iter().map(|t| energy_of(t, a.1, a.2)).sum();
                let eb: f64 = tracks.iter().map(|t| energy_of(t, b.1, b.2)).sum();
                ea.total_cmp(&eb)
            })
            .filter(|_| section_ranges.len() > 1)
        {
            if let Some(before) = section_ranges
                .iter()
                .find(|(_, _, end)| *end + 1 == peak.1)
            {
                notes.push(format!(
                    "Set up the climax: \"{}\" is the peak - strip the last 1-2 bars of \
                     \"{}\" (m{}) to bass and drums (delete or thin the guitar there) so \
                     the drop into m{} hits harder.",
                    peak.0, before.0, before.2, peak.1
                ));
            }
            if guitars.len() >= 1 {
                let lead = guitars
                    .iter()
                    .max_by(|a, b| a.mean_pitch.total_cmp(&b.mean_pitch))
                    .unwrap();
                notes.push(format!(
                    "Lift \"{}\": harmonize track {} \"{}\" in 3rds there \
                     (tuxguitar_generate_harmony, measures {}-{}) - instant dual-guitar \
                     chorus.",
                    peak.0, lead.number, lead.name, peak.1, peak.2
                ));
            }
        }
        let every_section_full = section_ranges.iter().all(|(_, lo, hi)| {
            melodic.iter().all(|t| energy_of(t, *lo, *hi) > 0.0)
        });
        if every_section_full && section_ranges.len() >= 3 {
            notes.push(
                "Every instrument plays in every section - give one section a reduced \
                 arrangement (bass carries, or drums+vocal-space only). Silence is an \
                 arrangement tool."
                    .to_string(),
            );
        }
        if let Some((label, lo, hi)) = section_ranges
            .iter()
            .find(|(label, _, _)| label.to_ascii_lowercase().contains("breakdown"))
        {
            notes.push(format!(
                "\"{}\" (m{}-{}): verify guitar/kick unison with tuxguitar_evaluate - \
                 breakdown weight comes from rhythmic alignment, not volume.",
                label, lo, hi
            ));
        }
        if notes.is_empty() {
            notes.push(
                "Arrangement reads balanced - nothing structural to flag. If it still \
                 feels flat, the problem is inside the riffs: run tuxguitar_hook_check."
                    .to_string(),
            );
        }
        let numbered: Vec<String> = notes
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{}. {}", i + 1, n))
            .collect();
        Ok(format!(
            "PRODUCER NOTES, measures {from}-{to} ({} sections):\n{}",
            section_ranges.len(),
            numbered.join("\n")
        ))
    }

    #[tool(
        description = "HOOK CHECK — the memorability gate: one critic whose only job is to reject riffs. Six criteria: a rememberable motif, a hummable contour, rhythmic identity, live dynamics, breathing room, and a surprise or developed repeat. Four of six passes; otherwise REJECTED with the reasons. Ask it: would anyone remember this tomorrow? If rejected, change one thing and re-run.",
        annotations(title = "Hook check", read_only_hint = true)
    )]
    async fn tuxguitar_hook_check(
        &self,
        params: Parameters<HookCheckParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let track_number = p.track_number;
        let (from, to) = (p.from_measure, p.to_measure);
        if from == 0 || to < from || to - from + 1 > MAX_MEASURES_PER_READ {
            return Err(ErrorData::invalid_params("invalid range", None));
        }
        let song = self.fetch_song().await?;
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track_number} not found"), None)
            })?;
        let tuning: Vec<(u32, u8)> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();
        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let report = tabmcp_theory::critique::critique(&range.measures, &tuning);
        let verdict = tabmcp_theory::critique::hook_check(&report);
        let mut out = format!(
            "HOOK CHECK - track {track_number} \"{}\", measures {from}-{to}: {}\n",
            track.name,
            if verdict.pass {
                "PASS - this could stick"
            } else {
                "REJECTED - would anyone remember this tomorrow?"
            }
        );
        for s in &verdict.strengths {
            out.push_str(&format!("  + {s}\n"));
        }
        for r in &verdict.rejections {
            out.push_str(&format!("  - {r}\n"));
        }
        if !verdict.pass {
            out.push_str(
                "Change ONE thing (vary a repeat, carve a rest, shape the contour) and \
                 re-run. Repeat until PASS.\n",
            );
        }
        Ok(out)
    }

    #[tool(
        description = "Set a section marker on a measure (e.g. 'Verse', 'Chorus') — visible in TuxGuitar and usable for song-structure navigation. Empty title clears the marker.",
        annotations(title = "Set marker", read_only_hint = false, destructive_hint = false)
    )]
    async fn tuxguitar_set_marker(
        &self,
        params: Parameters<SetMarkerParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let title = p.title.clone();
        self.call_bridge(move |client| client.set_marker(p.measure, &title))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(if p.title.is_empty() {
            format!("Marker cleared on measure {}.", p.measure)
        } else {
            format!("Marker \"{}\" set on measure {}.", p.title, p.measure)
        })
    }

    #[tool(
        description = "AI EAR — one deep listening pass over every track for the refinement loop: per-track groove consistency, syncopation, motif development (literal vs varied repeats), tension curve, harmonic rhythm, note density, dynamics (robotic-velocity detection), plus cross-track analysis (clashes, masking, alignment, empty bars), key/scale detection, a human-feel check for AI artifacts, and the session's pass-over-pass trend. Optional: style=<name> checks tempo+syncopation against that style's targets; tension_target=\"0.2,0.5,1.0\" compares the measured tension curve to a desired arc. Loop: evaluate -> fix the top issue -> evaluate again; finish with tuxguitar_render_and_listen.",
        annotations(title = "Evaluate (AI Ear)", read_only_hint = true)
    )]
    async fn tuxguitar_evaluate(
        &self,
        params: Parameters<EvaluateParams>,
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

        let mut out = format!("AI EAR scorecard, measures {from}-{to}:\n\n");
        let mut inputs = Vec::new();
        let mut all_events: Vec<NoteEvent> = Vec::new();
        let mut reports: Vec<(u32, bool, tabmcp_theory::critique::CritiqueReport)> = Vec::new();
        for track in &song.tracks {
            let track_number = track.number;
            let range = self
                .call_bridge(move |client| client.read_measures(track_number, from, to))
                .await
                .map_err(BridgeCallError::into_error_data)?;
            let tuning: Vec<(u32, u8)> = track
                .strings
                .iter()
                .map(|s| (s.number, s.open_pitch))
                .collect();
            if !track.is_percussion {
                let open: std::collections::HashMap<u32, u8> = tuning.iter().copied().collect();
                for measure in &range.measures {
                    for beat in &measure.beats {
                        for voice in &beat.voices {
                            for note in &voice.notes {
                                if let Some(&o) = open.get(&note.string) {
                                    all_events.push(NoteEvent {
                                        pitch: o.saturating_add(note.fret as u8),
                                        weight: 480,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            let report = tabmcp_theory::critique::critique(&range.measures, &tuning);
            out.push_str(&tabmcp_theory::critique::describe(
                &report,
                &format!("Track {} \"{}\"", track.number, track.name),
            ));
            reports.push((track.number, track.is_percussion, report));
            inputs.push(tabmcp_theory::arrangement::TrackInput {
                number: track.number,
                name: track.name.clone(),
                is_percussion: track.is_percussion,
                tuning,
                measures: range.measures,
            });
        }
        // Per-section groove (field finding: whole-range rhythm flags fire on
        // INTENTIONAL contrast between sections/meters). Sections split at
        // markers and time-signature changes.
        let mut boundaries: Vec<(u32, String)> = Vec::new();
        for (i, header) in song.headers.iter().enumerate() {
            let meter_changed =
                i > 0 && song.headers[i - 1].time_signature != header.time_signature;
            if header.number >= from
                && header.number <= to
                && (header.marker.is_some() || meter_changed || header.number == from)
            {
                let label = header.marker.clone().unwrap_or_else(|| {
                    format!(
                        "{}/{}",
                        header.time_signature.numerator, header.time_signature.denominator
                    )
                });
                if boundaries.last().map(|(m, _)| *m) != Some(header.number) {
                    boundaries.push((header.number, label));
                }
            }
        }
        if boundaries.len() > 1 {
            out.push_str("\nPer-section groove (whole-range rhythm flags above may just be intentional section contrast — trust these):\n");
            for (i, (start, label)) in boundaries.iter().enumerate() {
                let end = boundaries.get(i + 1).map(|(m, _)| m - 1).unwrap_or(to);
                let mut line = format!("  m{start}-{end} \"{label}\":");
                for input in &inputs {
                    let slice: Vec<tabmcp_model::Measure> = input
                        .measures
                        .iter()
                        .filter(|m| m.number >= *start && m.number <= end)
                        .cloned()
                        .collect();
                    if slice.iter().all(|m| m.beats.is_empty()) {
                        continue;
                    }
                    let report = tabmcp_theory::critique::critique(&slice, &input.tuning);
                    line.push_str(&format!(
                        " T{} {:.0}%",
                        input.number,
                        report.groove_consistency * 100.0
                    ));
                }
                line.push('\n');
                out.push_str(&line);
            }
        }

        out.push('\n');
        let arrangement = tabmcp_theory::arrangement::analyze_arrangement(&inputs);
        out.push_str(&tabmcp_theory::arrangement::describe(&arrangement));
        if let Some(best) = detect_scales(&all_events).first() {
            out.push_str(&format!(
                "Key/scale: {} {} (confidence {:.0}%)\n",
                best.root,
                best.scale,
                best.confidence * 100.0
            ));
        }
        // Style check: compare tempo + per-track syncopation to the guide's
        // numeric targets.
        if let Some(style_name) = &p.style {
            let guide = tabmcp_theory::styles::style_guide(style_name).ok_or_else(|| {
                ErrorData::invalid_params(
                    format!(
                        "unknown style '{style_name}'; available: {}",
                        tabmcp_theory::styles::STYLES
                            .iter()
                            .map(|s| s.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    None,
                )
            })?;
            out.push_str(&format!("\nSTYLE CHECK ({}):\n", guide.name));
            if let Some(header) = song.headers.iter().find(|h| h.number == from) {
                let bpm = header.tempo_bpm;
                let ok = bpm >= guide.tempo_range.0 as u32 && bpm <= guide.tempo_range.1 as u32;
                out.push_str(&format!(
                    "  tempo {bpm} BPM {} target {}-{}\n",
                    if ok { "OK, within" } else { "OUTSIDE" },
                    guide.tempo_range.0,
                    guide.tempo_range.1
                ));
            }
            for (number, is_percussion, report) in &reports {
                if *is_percussion {
                    continue;
                }
                let s = report.syncopation;
                let (lo, hi) = guide.syncopation;
                let verdict = if s < lo {
                    "below target - too metronomic for the style"
                } else if s > hi {
                    "above target - too unmoored for the style"
                } else {
                    "OK, in the style's window"
                };
                out.push_str(&format!(
                    "  T{number} syncopation {:.0}% {} ({:.0}-{:.0}%)\n",
                    s * 100.0,
                    verdict,
                    lo * 100.0,
                    hi * 100.0
                ));
            }
        }

        // Emotion journey resolves to a tension target with labels.
        let emotion = match &p.emotion_target {
            Some(spec) => Some(
                tabmcp_theory::emotion::curve(spec)
                    .map_err(|e| ErrorData::invalid_params(e, None))?,
            ),
            None => None,
        };

        // Tension target: compare the mean measured curve to the desired arc.
        let tension_spec = p
            .tension_target
            .clone()
            .or_else(|| {
                emotion.as_ref().map(|(values, _)| {
                    values
                        .iter()
                        .map(|v| format!("{v:.2}"))
                        .collect::<Vec<_>>()
                        .join(",")
                })
            });
        if let Some(target_spec) = &tension_spec {
            let target: Vec<f64> = target_spec
                .split(',')
                .map(|s| s.trim().parse::<f64>())
                .collect::<Result<_, _>>()
                .map_err(|_| {
                    ErrorData::invalid_params(
                        format!("bad tension_target '{target_spec}'; expected e.g. \"0.2,0.5,1.0\""),
                        None,
                    )
                })?;
            let n = (to - from + 1) as usize;
            let curves: Vec<&Vec<f64>> = reports
                .iter()
                .filter(|(_, perc, r)| !perc && r.tension.len() == n)
                .map(|(_, _, r)| &r.tension)
                .collect();
            if !target.is_empty() && !curves.is_empty() {
                let mean_curve: Vec<f64> = (0..n)
                    .map(|i| curves.iter().map(|c| c[i]).sum::<f64>() / curves.len() as f64)
                    .collect();
                let mut worst = (0usize, 0.0f64, 0.0f64);
                let mut total_gap = 0.0;
                for (i, &measured) in mean_curve.iter().enumerate() {
                    let wanted = target[(i * target.len()) / n];
                    let gap = (measured - wanted).abs();
                    total_gap += gap;
                    if gap > (worst.1 - worst.2).abs() {
                        worst = (i, measured, wanted);
                    }
                }
                let wanted_label = emotion
                    .as_ref()
                    .and_then(|(_, labels)| labels.get((worst.0 * target.len()) / n))
                    .map(|l| format!(" (should feel \"{l}\")"))
                    .unwrap_or_default();
                let header = if emotion.is_some() {
                    "EMOTION JOURNEY"
                } else {
                    "TENSION TARGET"
                };
                out.push_str(&format!(
                    "\n{header}: mean deviation {:.0}%; largest gap at measure {}{} \
                     (wanted {:.1}, measured {:.1}). Raise tension with density/velocity/\
                     register/dissonance, lower it with space.\n",
                    total_gap / n as f64 * 100.0,
                    from + worst.0 as u32,
                    wanted_label,
                    worst.2,
                    worst.1
                ));
            }
        }

        // Human-feel check: telltale AI artifacts across the arrangement.
        let mut artifacts: Vec<&str> = Vec::new();
        let melodic: Vec<&tabmcp_theory::critique::CritiqueReport> = reports
            .iter()
            .filter(|(_, perc, _)| !perc)
            .map(|(_, _, r)| r)
            .collect();
        if !melodic.is_empty() {
            if melodic.iter().all(|r| r.velocity_std < 2.0) {
                artifacts.push("identical velocities everywhere");
            }
            if melodic
                .iter()
                .all(|r| r.literal_repeats >= 2 && r.varied_repeats == 0)
            {
                artifacts.push("only literal copy-paste repeats, zero development");
            }
            if melodic.iter().all(|r| {
                r.tension.len() >= 4 && {
                    let mean = r.tension.iter().sum::<f64>() / r.tension.len() as f64;
                    (r.tension.iter().map(|t| (t - mean).powi(2)).sum::<f64>()
                        / r.tension.len() as f64)
                        .sqrt()
                        < 0.08
                }
            }) {
                artifacts.push("constant density/energy with no build or release");
            }
            if melodic
                .iter()
                .all(|r| r.motif_repetition < 0.15 && r.fresh_measures > r.varied_repeats)
            {
                artifacts.push("no memorable motif - material wanders without returning");
            }
        }
        if artifacts.is_empty() {
            out.push_str("\nHUMAN-FEEL CHECK: passes - no obvious AI artifacts.\n");
        } else {
            out.push_str(&format!(
                "\nHUMAN-FEEL CHECK: {} artifact(s) - {}. Revise until it sounds like a \
                 guitarist wrote it (humanize, vary a repeat, shape a build).\n",
                artifacts.len(),
                artifacts.join("; ")
            ));
        }

        // Pass history: session trend across evaluate calls.
        {
            let mean_groove = if melodic.is_empty() {
                0.0
            } else {
                melodic.iter().map(|r| r.groove_consistency).sum::<f64>() / melodic.len() as f64
            };
            let issues = out.matches("ISSUE:").count();
            let mut passes = self.passes.lock().unwrap();
            passes.push((mean_groove, issues));
            if passes.len() > 1 {
                let grooves: Vec<String> = passes
                    .iter()
                    .map(|(g, _)| format!("{:.0}%", g * 100.0))
                    .collect();
                let issue_counts: Vec<String> =
                    passes.iter().map(|(_, i)| i.to_string()).collect();
                out.push_str(&format!(
                    "\nPASS TREND (#{}): groove {} | issues {}\n",
                    passes.len(),
                    grooves.join(" -> "),
                    issue_counts.join(" -> ")
                ));
            }
        }

        out.push_str(
            "\nNext: fix the top ISSUE with the edit tools (each fix is undoable), then \
             evaluate again; when the scorecard is clean, run tuxguitar_render_and_listen.",
        );
        Ok(out)
    }

    #[tool(
        description = "RIFF DNA — extract the musical identity of a riff: motif (recurring interval pattern), rhythm cell (dominant IOIs + syncopation), scale, register, technique mix, energy 1-10, and harmonic motion. Use the DNA to generate NEW material that keeps the identity without copying: same rhythm cell + scale + energy, different pitch order — or feed it to tuxguitar_vary_riff for mechanical mutations. This is how you evolve riffs instead of replacing them.",
        annotations(title = "Riff DNA", read_only_hint = true)
    )]
    async fn tuxguitar_riff_dna(
        &self,
        params: Parameters<RiffDnaParams>,
    ) -> Result<String, ErrorData> {
        let Parameters(p) = params;
        let bank_path = PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".tuxguitar-mcp")
            .join("dna_bank.jsonl");
        if p.list_bank {
            let listing = std::fs::read_to_string(&bank_path).unwrap_or_default();
            if listing.trim().is_empty() {
                return Ok("DNA bank is empty - extract with riff_dna save_as=<name>.".into());
            }
            let mut out = String::from("DNA BANK (saved riff identities):\n");
            for line in listing.lines() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    out.push_str(&format!(
                        "--- {} ---\n{}\n",
                        v.get("name").and_then(|n| n.as_str()).unwrap_or("?"),
                        v.get("card").and_then(|c| c.as_str()).unwrap_or(""),
                    ));
                }
            }
            return Ok(out);
        }
        let (Some(track_number), Some(from), Some(to)) =
            (p.track_number, p.from_measure, p.to_measure)
        else {
            return Err(ErrorData::invalid_params(
                "track_number, from_measure and to_measure are required (or pass list_bank=true)",
                None,
            ));
        };
        if from == 0 || to < from || to - from + 1 > MAX_MEASURES_PER_READ {
            return Err(ErrorData::invalid_params("invalid range", None));
        }
        let song = self.fetch_song().await?;
        let track = song
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("track {track_number} not found"), None)
            })?;
        let tuning: Vec<(u32, u8)> = track
            .strings
            .iter()
            .map(|s| (s.number, s.open_pitch))
            .collect();
        let range = self
            .call_bridge(move |client| client.read_measures(track_number, from, to))
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let report = tabmcp_theory::critique::critique(&range.measures, &tuning);

        // Pitch + rhythm + technique scan.
        let open: std::collections::HashMap<u32, u8> = tuning.iter().copied().collect();
        let mut pitches: Vec<u8> = Vec::new();
        let mut offsets: Vec<u64> = Vec::new();
        let mut note_count = 0usize;
        let mut technique_counts: Vec<(&str, usize)> = vec![
            ("palmMute", 0),
            ("staccato", 0),
            ("letRing", 0),
            ("deadNote", 0),
            ("ghostNote", 0),
            ("vibrato", 0),
            ("bend", 0),
            ("harmonic", 0),
            ("hammer/legato", 0),
            ("slide", 0),
        ];
        for measure in &range.measures {
            for beat in &measure.beats {
                for voice in &beat.voices {
                    for note in &voice.notes {
                        if note.tied {
                            continue;
                        }
                        note_count += 1;
                        if let Some(&o) = open.get(&note.string) {
                            pitches.push(o.saturating_add(note.fret as u8));
                            offsets.push(beat.start_tick);
                        }
                        let e = &note.effects;
                        for (name, count) in technique_counts.iter_mut() {
                            let hit = match *name {
                                "palmMute" => e.palm_mute,
                                "staccato" => e.staccato,
                                "letRing" => e.let_ring,
                                "deadNote" => e.dead_note,
                                "ghostNote" => e.ghost_note,
                                "vibrato" => e.vibrato,
                                "bend" => e.bend.is_some(),
                                "harmonic" => e.harmonic.is_some(),
                                "hammer/legato" => e.hammer,
                                "slide" => e.slide,
                                _ => false,
                            };
                            if hit {
                                *count += 1;
                            }
                        }
                    }
                }
            }
        }
        if note_count == 0 {
            return Ok(format!(
                "RIFF DNA (track {track_number}, measures {from}-{to}): empty range - nothing to sequence."
            ));
        }

        // Scale from this riff's own pitches.
        let events: Vec<NoteEvent> = pitches
            .iter()
            .map(|&pitch| NoteEvent { pitch, weight: 480 })
            .collect();
        let scale_line = detect_scales(&events)
            .first()
            .map(|s| format!("{} {} ({:.0}%)", s.root, s.scale, s.confidence * 100.0))
            .unwrap_or_else(|| "ambiguous".into());

        // Dominant rhythm cell: the 2-3 most common IOIs.
        let mut ioi_counts: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for pair in offsets.windows(2) {
            let gap = pair[1].saturating_sub(pair[0]);
            if gap > 0 {
                *ioi_counts.entry(gap).or_default() += 1;
            }
        }
        let mut iois: Vec<(u64, usize)> = ioi_counts.into_iter().collect();
        iois.sort_by(|a, b| b.1.cmp(&a.1));
        let cell: Vec<String> = iois
            .iter()
            .take(3)
            .map(|(gap, count)| format!("{gap}t x{count}"))
            .collect();

        // Energy 1-10: density + velocity + register drive.
        let measures_n = (to - from + 1) as f64;
        let density = note_count as f64 / measures_n;
        let energy = ((density / 16.0 * 5.0)
            + (report.velocity_mean / 127.0 * 3.0)
            + (report.syncopation * 2.0))
            .clamp(1.0, 10.0);

        let lo = pitches.iter().min().copied().unwrap_or(0);
        let hi = pitches.iter().max().copied().unwrap_or(0);
        let techniques: Vec<String> = technique_counts
            .iter()
            .filter(|(_, count)| *count > 0)
            .map(|(name, count)| {
                format!("{name} {:.0}%", *count as f64 / note_count as f64 * 100.0)
            })
            .collect();

        let card = format!(
            "RIFF DNA (track {track_number} \"{}\", measures {from}-{to}):\n\
             Motif: interval pattern {:?} (covers {:.0}% of the line)\n\
             Rhythm cell: {} | syncopation {:.0}% | groove {:.0}%\n\
             Scale: {scale_line}\n\
             Register: MIDI {lo}-{hi} ({}-{})\n\
             Techniques: {}\n\
             Energy: {energy:.1}/10 ({density:.1} notes/measure, velocity {:.0})\n\
             Harmonic motion: root changes {:.0}% of measures\n\n\
             To EVOLVE this riff (not copy it): keep the rhythm cell + scale + energy, \
             write NEW pitches from the same scale in the same register; or keep the \
             motif and displace/regroup the rhythm (tuxguitar_vary_riff). Change at most \
             two DNA strands per generation so the family resemblance survives.",
            track.name,
            report.top_motif,
            report.motif_repetition * 100.0,
            if cell.is_empty() {
                "none (single onset)".into()
            } else {
                cell.join(" ")
            },
            report.syncopation * 100.0,
            report.groove_consistency * 100.0,
            tabmcp_theory::pitch::note_name(lo),
            tabmcp_theory::pitch::note_name(hi),
            if techniques.is_empty() {
                "none (dry picking)".into()
            } else {
                techniques.join(", ")
            },
            report.velocity_mean,
            report.harmonic_rhythm * 100.0,
        );
        if let Some(name) = &p.save_as {
            let entry = serde_json::json!({ "name": name, "card": card });
            if let Some(parent) = bank_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            use std::io::Write as _;
            let saved = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&bank_path)
                .and_then(|mut f| writeln!(f, "{entry}"));
            return Ok(match saved {
                Ok(()) => format!("{card}\nSaved to the DNA bank as \"{name}\"."),
                Err(e) => format!("{card}\nWARNING: could not save to the DNA bank: {e}"),
            });
        }
        Ok(card)
    }

    #[tool(
        description = "AI EAR stems: render EACH track to its own audio file and analyze them separately (per-track loudness, spectral balance, clipping) — hears which instrument causes mud or imbalance, which the full-mix render can't isolate. Slower: one synth render per track. Stems kept at ~/.tuxguitar-mcp/stems/ for the user.",
        annotations(title = "Listen to stems", read_only_hint = true)
    )]
    async fn tuxguitar_listen_stems(&self) -> Result<String, ErrorData> {
        let song = self.fetch_song().await?;
        let midi = self
            .call_bridge(|client| client.render_midi())
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let midi_path = PathBuf::from(
            midi.get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| ErrorData::internal_error("bridge returned no render path", None))?,
        );
        let home = std::env::var("HOME").unwrap_or_default();
        let stems_dir = PathBuf::from(&home).join(".tuxguitar-mcp/stems");

        let track_names: Vec<String> = song
            .tracks
            .iter()
            .map(|t| format!("{} \"{}\"", t.number, t.name))
            .collect();
        let report =
            tokio::task::spawn_blocking(move || -> Result<String, String> {
                let stems = crate::render::split_stems(&midi_path, &stems_dir)?;
                let mut out = String::new();
                for (i, stem) in stems.iter().enumerate() {
                    let wav = stem.with_extension("wav");
                    crate::render::render_wav(stem, &wav)?;
                    let analysis = crate::audio::analyze_wav(&wav)?;
                    let (low, mid, high) = analysis.band_share;
                    let label = track_names
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("stem {}", i + 1));
                    out.push_str(&format!(
                    "Track {label}: peak {:.1} dBFS, RMS {:.1} dBFS, spectrum {:.0}/{:.0}/{:.0}% \
                     (low/mid/high){}\n",
                    analysis.peak_dbfs,
                    analysis.rms_dbfs,
                    low * 100.0,
                    mid * 100.0,
                    high * 100.0,
                    if analysis.clipped_samples > 0 { " CLIPPING" } else { "" },
                ));
                    if analysis.peak_dbfs < -60.0 {
                        out.push_str(
                            "  PRESCRIPTION: essentially silent in isolation - notes likely sit \
                         below the soundfont sampled range; transpose the part up an octave\n",
                        );
                    }
                }
                Ok(out)
            })
            .await
            .map_err(|e| ErrorData::internal_error(format!("stem task failed: {e}"), None))?
            .map_err(|e| ErrorData::internal_error(e, None))?;

        let healthy = if report.contains("PRESCRIPTION") {
            ""
        } else {
            "No prescriptions — all stems healthy (audible, unclipped).\n"
        };
        Ok(format!(
            "Per-track stems:\n{report}{healthy}Stems kept in ~/.tuxguitar-mcp/stems/ (mid + wav per track).",
        ))
    }

    #[tool(
        description = "The 'virtual ear', audio edition: render the WHOLE song through TuxGuitar's own soundfont (headless MIDI -> fluidsynth -> WAV) and analyze the actual audio — true loudness, clipping, spectral balance (low-end mud / darkness), and quiet holes. Slower than tuxguitar_analyze_arrangement (use that for note-level issues); use this to hear the MIX. Requires fluidsynth (brew install fluid-synth). The WAV is kept at ~/.tuxguitar-mcp/render.wav for the user to play.",
        annotations(title = "Render & listen", read_only_hint = true)
    )]
    async fn tuxguitar_render_and_listen(&self) -> Result<String, ErrorData> {
        // 1. Headless MIDI from the bridge (fixed scratch path, no dialogs).
        let midi = self
            .call_bridge(|client| client.render_midi())
            .await
            .map_err(BridgeCallError::into_error_data)?;
        let midi_path = midi
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ErrorData::internal_error("bridge returned no render path", None))?
            .to_string();

        // 2. Locate fluidsynth and a soundfont (user override first, then the
        //    one TuxGuitar itself ships, so it sounds like the app's playback).
        let fluidsynth = [
            "/opt/homebrew/bin/fluidsynth",
            "/usr/local/bin/fluidsynth",
            "fluidsynth",
        ]
        .iter()
        .find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok()
        })
        .ok_or_else(|| {
            ErrorData::internal_error(
                "fluidsynth not found — install it with: brew install fluid-synth",
                None,
            )
        })?;
        let home = std::env::var("HOME").unwrap_or_default();
        let override_font = PathBuf::from(&home).join(".tuxguitar-mcp/soundfont.sf2");
        let soundfont = if override_font.exists() {
            override_font
        } else {
            let mut found = None;
            if let Ok(apps) = std::fs::read_dir("/Applications") {
                for app in apps.flatten() {
                    if app
                        .file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains("tuxguitar")
                    {
                        let candidate = app
                            .path()
                            .join("Contents/MacOS/share/soundfont/MagicSFver2.sf2");
                        if candidate.exists() {
                            found = Some(candidate);
                        }
                    }
                }
            }
            found.ok_or_else(|| {
                ErrorData::internal_error(
                    "no soundfont found — place a GM .sf2 at ~/.tuxguitar-mcp/soundfont.sf2",
                    None,
                )
            })?
        };

        // 3. Render to WAV.
        let wav_path = PathBuf::from(&home).join(".tuxguitar-mcp/render.wav");
        let output = tokio::task::spawn_blocking({
            let fluidsynth = fluidsynth.to_string();
            let soundfont = soundfont.clone();
            let midi_path = midi_path.clone();
            let wav_path = wav_path.clone();
            move || {
                std::process::Command::new(fluidsynth)
                    .args(["-ni", "-r", "44100", "-F"])
                    .arg(&wav_path)
                    .arg(&soundfont)
                    .arg(&midi_path)
                    .output()
            }
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("render task failed: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("fluidsynth failed to start: {e}"), None))?;
        if !output.status.success() || !wav_path.exists() {
            return Err(ErrorData::internal_error(
                format!(
                    "fluidsynth render failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                        .chars()
                        .take(300)
                        .collect::<String>()
                ),
                None,
            ));
        }

        // 4. Listen.
        let report = tokio::task::spawn_blocking({
            let wav_path = wav_path.clone();
            move || crate::audio::analyze_wav(&wav_path)
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("analysis task failed: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(e, None))?;

        // Measure-aligned levels when the timeline is linear (no repeats).
        let mut measure_notes = String::new();
        let song = self.fetch_song().await.ok();
        if let Some(song) = song {
            let has_repeats = song
                .headers
                .iter()
                .any(|h| h.repeat_open || h.repeat_close > 0);
            if !has_repeats && song.headers.len() > 1 {
                let mut boundaries = vec![0.0f64];
                let mut t = 0.0f64;
                for (i, header) in song.headers.iter().enumerate() {
                    let len_ticks = song
                        .headers
                        .get(i + 1)
                        .map(|n| n.start_tick.saturating_sub(header.start_tick))
                        .unwrap_or_else(|| {
                            header.time_signature.numerator as u64 * 960 * 4
                                / header.time_signature.denominator.max(1) as u64
                        });
                    t += len_ticks as f64 / 960.0 * 60.0 / header.tempo_bpm.max(1) as f64;
                    boundaries.push(t);
                }
                if let Ok(levels) = tokio::task::spawn_blocking({
                    let wav_path = wav_path.clone();
                    move || crate::audio::measure_levels(&wav_path, &boundaries)
                })
                .await
                .unwrap_or_else(|_| Err("task".into()))
                {
                    if let (Some(max_i), Some(min_i)) = (
                        levels
                            .iter()
                            .enumerate()
                            .max_by(|a, b| a.1.total_cmp(b.1))
                            .map(|(i, _)| i),
                        levels
                            .iter()
                            .enumerate()
                            .min_by(|a, b| a.1.total_cmp(b.1))
                            .map(|(i, _)| i),
                    ) {
                        measure_notes = format!(
                            "Measure levels: loudest m{} ({:.1} dBFS), quietest m{} ({:.1} dBFS)\n",
                            max_i + 1,
                            levels[max_i],
                            min_i + 1,
                            levels[min_i]
                        );
                    }
                }
            }
        }
        Ok(format!(
            "{}{}Rendered with TuxGuitar's soundfont ({}).\nAudio kept at {} — the user can play it.",
            crate::audio::describe(&report),
            measure_notes,
            soundfont.file_name().and_then(|n| n.to_str()).unwrap_or("sf2"),
            wav_path.display(),
        ))
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
    Counterline,
    InterlockDrums,
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
                let style = p.style.clone().unwrap_or_else(|| "rock".into());
                let (measures, description) = tabmcp_theory::generation::generate_drums(
                    &range.measures,
                    &source_tuning,
                    &style,
                )
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
            GenerateKind::Counterline => {
                let (measures, description) = tabmcp_theory::generation::generate_counterline(
                    &range.measures,
                    &source_tuning,
                    max_fret,
                )
                .map_err(|e| ErrorData::invalid_params(e, None))?;
                let strings: Vec<tabmcp_model::StringTuning> = track.strings.clone();
                (
                    "Lead Guitar (AI)".to_string(),
                    strings,
                    measures,
                    description,
                    None::<&str>,
                )
            }
            GenerateKind::InterlockDrums => {
                let (measures, description) =
                    tabmcp_theory::generation::generate_interlock_drums(
                        &range.measures,
                        &source_tuning,
                    )
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
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
        let percussion = matches!(kind, GenerateKind::Drums | GenerateKind::InterlockDrums);
        let note_count = count_notes(&generated);

        if !p.confirm {
            let destination = match p.target_track {
                Some(existing) => format!("EXISTING track {existing}"),
                None => format!("a new track \"{new_track_name}\""),
            };
            return Ok(Json(EditOutcome {
                applied: false,
                summary: format!(
                    "PREVIEW ONLY — nothing changed. Would write into {destination} \
                     {note_count} notes across measures {from}-{to}: {description}. \
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

        let (new_track, post_create_revision) = if let Some(existing) = p.target_track {
            if !song.tracks.iter().any(|t| t.number == existing) {
                return Err(ErrorData::invalid_params(
                    format!("target_track {existing} does not exist"),
                    None,
                ));
            }
            (existing, expected_revision)
        } else {
            let name_for_create = new_track_name.clone();
            let created = self
                .call_bridge(move |client| {
                    client.create_track(&name_for_create, &target_strings, clef, percussion)
                })
                .await
                .map_err(BridgeCallError::into_error_data)?;
            (
                created
                    .get("trackNumber")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as u32,
                created
                    .get("newRevision")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            )
        };

        let result = self
            .call_bridge(move |client| {
                client.apply_replace_measures(new_track, from, &generated, post_create_revision)
            })
            .await
            .map_err(BridgeCallError::into_error_data)?;
        Ok(Json(EditOutcome {
            applied: true,
            summary: format!(
                "Applied: wrote {} notes into {} (measures {from}-{to}) — {description}.{}",
                result.notes_after,
                match p.target_track {
                    Some(existing) => format!("existing track {existing}"),
                    None => format!("new track {new_track} \"{new_track_name}\""),
                },
                if p.target_track.is_some() {
                    " Undoable with Cmd+Z."
                } else {
                    " Undo takes two Cmd+Z steps."
                },
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

    /// Read an arbitrary measure range by chunking bridge reads at the
    /// per-call limit.
    async fn read_measures_chunked(
        &self,
        track_number: u32,
        from: u32,
        to: u32,
    ) -> Result<Vec<tabmcp_model::Measure>, ErrorData> {
        let mut out = Vec::new();
        let mut cursor = from;
        while cursor <= to {
            let end = (cursor + MAX_MEASURES_PER_READ - 1).min(to);
            let range = self
                .call_bridge(move |client| client.read_measures(track_number, cursor, end))
                .await
                .map_err(BridgeCallError::into_error_data)?;
            out.extend(range.measures);
            cursor = end + 1;
        }
        Ok(out)
    }

    /// Song sections between `from..=to`: split at markers and time-signature
    /// changes; always starts at `from`. Returns (start_measure, label).
    fn section_boundaries(song: &Song, from: u32, to: u32) -> Vec<(u32, String)> {
        let mut boundaries: Vec<(u32, String)> = Vec::new();
        for (i, header) in song.headers.iter().enumerate() {
            let meter_changed =
                i > 0 && song.headers[i - 1].time_signature != header.time_signature;
            if header.number >= from
                && header.number <= to
                && (header.marker.is_some() || meter_changed || header.number == from)
            {
                let label = header.marker.clone().unwrap_or_else(|| {
                    format!(
                        "{}/{}",
                        header.time_signature.numerator, header.time_signature.denominator
                    )
                });
                if boundaries.last().map(|(m, _)| *m) != Some(header.number) {
                    boundaries.push((header.number, label));
                }
            }
        }
        boundaries
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
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for TabMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_prompts()
            .build();
        info.instructions = Some(
            "TabMCP connects you to the score currently open in the TuxGuitar tablature \
                 editor. Typical flow: tuxguitar_get_score_summary for orientation, \
                 tuxguitar_get_selection to see what the user highlighted, then \
                 tuxguitar_get_measures / tuxguitar_explain_selection / \
                 tuxguitar_detect_key_and_scale for content and analysis. If tools report the \
                 bridge is unavailable, the user needs to start TuxGuitar with the TabMCP \
                 plugin installed. String numbers are 1-based (1 = highest string); pitch = \
                 open-string pitch + fret; time is in ticks (960 per quarter note). \
                 AI-EAR REFINEMENT LOOP: after composing or generating, call \
                 tuxguitar_evaluate for a scorecard, fix the top issue with the edit \
                 tools (every fix previews first and is undoable), re-evaluate, and \
                 finish with tuxguitar_render_and_listen to hear the real mix. Each \
                 pass, tell the user what changed and why. The undo stack is the \
                 version history: Cmd+Z steps back through your passes."
                .into(),
        );
        info
    }
}
