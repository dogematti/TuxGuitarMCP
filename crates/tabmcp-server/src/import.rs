//! MIDI import: parse a Standard MIDI File, quantize onto the tick grid,
//! and produce editor-ready measures (string/fret assignment is done by the
//! caller via the fingering optimizer).

use std::collections::BTreeMap;
use std::path::Path;

use tabmcp_model::{Beat, Duration, Measure, Note, NoteEffects, Tuplet, Voice, TICKS_PER_QUARTER};

/// (quantized tick, (pitches, max velocity, min raw duration))
type Moment = (u64, (Vec<u8>, u32, u64));

/// One imported (possibly chordal) moment after quantization.
pub struct ImportedStep {
    /// Measure-relative offset in our ticks.
    pub offset: u64,
    pub measure_index: usize,
    /// Pitches sounding at this moment, ascending.
    pub pitches: Vec<u8>,
    pub velocity: u32,
    pub duration_value: u32,
}

pub struct ImportedSong {
    pub steps: Vec<ImportedStep>,
    pub measure_count: usize,
    pub note_count: usize,
    /// Which content track was imported (1-based) and what was available.
    pub chosen_track: usize,
    pub available_tracks: Vec<(usize, usize)>, // (index, note count)
}

/// Parse + quantize a MIDI file. `grid` is the quantization denominator
/// (16 = sixteenth grid). Percussion (channel 9) is skipped. Assumes 4/4.
pub fn parse_midi(
    path: &Path,
    grid: u32,
    midi_track: Option<usize>,
) -> Result<ImportedSong, String> {
    let bytes = std::fs::read(path).map_err(|e| {
        format!(
            "cannot read {} — {e}. Put the file there and retry.",
            path.display()
        )
    })?;
    let smf = midly::Smf::parse(&bytes).map_err(|e| format!("not a valid MIDI file: {e}"))?;
    let ticks_per_beat = match smf.header.timing {
        midly::Timing::Metrical(t) => t.as_int() as u64,
        midly::Timing::Timecode(..) => return Err("SMPTE-timed MIDI is not supported".into()),
    };

    // Count melodic notes per MIDI track; import ONE track (multi-track
    // merges produce unplayable cross-instrument chords).
    let note_ons = |track: &midly::Track| -> usize {
        track
            .iter()
            .filter(|e| {
                matches!(e.kind,
                    midly::TrackEventKind::Midi { channel, message: midly::MidiMessage::NoteOn { vel, .. } }
                        if vel.as_int() > 0 && channel.as_int() != 9)
            })
            .count()
    };
    let available_tracks: Vec<(usize, usize)> = smf
        .tracks
        .iter()
        .map(note_ons)
        .enumerate()
        .filter(|(_, n)| *n > 0)
        .enumerate()
        .map(|(content_index, (_, count))| (content_index + 1, count))
        .collect();
    if available_tracks.is_empty() {
        return Err(
            "the MIDI file contains no melodic notes (channel-10 drums are skipped)".into(),
        );
    }
    let content_indices: Vec<usize> = smf
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| note_ons(t) > 0)
        .map(|(i, _)| i)
        .collect();
    let chosen_track = midi_track
        .unwrap_or_else(|| {
            // Default: the densest track.
            available_tracks
                .iter()
                .max_by_key(|(_, n)| *n)
                .map(|(i, _)| *i)
                .unwrap_or(1)
        })
        .clamp(1, content_indices.len());
    let target_smf_track = content_indices[chosen_track - 1];

    let step_ticks = (TICKS_PER_QUARTER * 4 / grid.max(1) as u64).max(1);
    // (quantized_tick) -> (pitches, max velocity, min raw duration)
    let mut moments: BTreeMap<u64, (Vec<u8>, u32, u64)> = BTreeMap::new();
    let mut note_count = 0usize;

    for (track_index, track) in smf.tracks.iter().enumerate() {
        if track_index != target_smf_track {
            continue;
        }
        let mut absolute = 0u64;
        let mut active: BTreeMap<(u8, u8), (u64, u8)> = BTreeMap::new(); // (ch,key) -> (start, vel)
        for event in track {
            absolute += event.delta.as_int() as u64;
            if let midly::TrackEventKind::Midi { channel, message } = event.kind {
                if channel.as_int() == 9 {
                    continue; // percussion imports are a later feature
                }
                match message {
                    midly::MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                        active.insert((channel.as_int(), key.as_int()), (absolute, vel.as_int()));
                    }
                    midly::MidiMessage::NoteOn { key, .. }
                    | midly::MidiMessage::NoteOff { key, .. } => {
                        if let Some((start, vel)) = active.remove(&(channel.as_int(), key.as_int()))
                        {
                            let our_start = start * TICKS_PER_QUARTER / ticks_per_beat;
                            let our_end = absolute * TICKS_PER_QUARTER / ticks_per_beat;
                            let quantized = (our_start + step_ticks / 2) / step_ticks * step_ticks;
                            let duration = our_end.saturating_sub(our_start).max(step_ticks);
                            let entry = moments
                                .entry(quantized)
                                .or_insert_with(|| (Vec::new(), 0, u64::MAX));
                            if !entry.0.contains(&key.as_int()) {
                                entry.0.push(key.as_int());
                                note_count += 1;
                            }
                            entry.1 = entry.1.max(vel as u32);
                            entry.2 = entry.2.min(duration);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if moments.is_empty() {
        return Err(
            "the MIDI file contains no melodic notes (channel-10 drums are skipped)".into(),
        );
    }

    let measure_ticks = TICKS_PER_QUARTER * 4; // 4/4 assumption, documented
    let mut steps = Vec::new();
    let mut max_measure = 0usize;
    let entries: Vec<Moment> = moments.into_iter().collect();
    for (i, (tick, (mut pitches, velocity, duration))) in entries.iter().cloned().enumerate() {
        pitches.sort_unstable();
        let measure_index = (tick / measure_ticks) as usize;
        max_measure = max_measure.max(measure_index);
        // Cap duration at the gap to the next moment so voices don't overlap.
        let gap = entries
            .get(i + 1)
            .map(|(next, _)| next - tick)
            .unwrap_or(duration);
        let effective = duration.min(gap).max(step_ticks);
        // Nearest plain note value (1..64) for the quantized duration.
        let duration_value = [1u32, 2, 4, 8, 16, 32, 64]
            .into_iter()
            .min_by_key(|v| {
                let ticks = TICKS_PER_QUARTER * 4 / *v as u64;
                ticks.abs_diff(effective)
            })
            .unwrap_or(8);
        steps.push(ImportedStep {
            offset: tick % measure_ticks,
            measure_index,
            pitches,
            velocity,
            duration_value,
        });
    }
    Ok(ImportedSong {
        steps,
        measure_count: max_measure + 1,
        note_count,
        chosen_track,
        available_tracks,
    })
}

/// Build measures from imported steps + optimizer-chosen positions
/// (one position set per step, aligned to ascending pitches).
pub fn build_measures(
    song: &ImportedSong,
    positions: &[Vec<tabmcp_theory::fingering::Position>],
) -> Vec<Measure> {
    let mut measures: Vec<Measure> = (0..song.measure_count)
        .map(|i| Measure {
            number: i as u32 + 1,
            start_tick: 0,
            key_signature: 0,
            beats: Vec::new(),
        })
        .collect();
    for (step, set) in song.steps.iter().zip(positions) {
        measures[step.measure_index].beats.push(Beat {
            start_tick: step.offset,
            voices: vec![Voice {
                index: 0,
                duration: Duration {
                    value: step.duration_value,
                    dotted: false,
                    double_dotted: false,
                    tuplet: Tuplet {
                        enters: 1,
                        times: 1,
                    },
                },
                is_rest: false,
                notes: set
                    .iter()
                    .map(|p| Note {
                        string: p.string_number,
                        fret: p.fret,
                        velocity: step.velocity,
                        tied: false,
                        effects: NoteEffects::default(),
                    })
                    .collect(),
            }],
        });
    }
    measures
}
