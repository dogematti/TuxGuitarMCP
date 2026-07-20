//! Phase 8 generation: bassline and harmony lines derived from existing
//! material. Both generators return editor-ready `Measure`s: pitches are
//! chosen musically, then mapped to string/fret via the fingering optimizer,
//! and the rhythm mirrors the source material's onsets.

use tabmcp_model::{Beat, Duration, Measure, Note, NoteEffects, Tuplet, Voice};

use crate::analysis::{detect_scales, scale_pitch_classes, NoteEvent};
use crate::fingering::{optimize_monophonic, CostModel, Tuning};

/// One melodic onset extracted from source measures.
#[derive(Debug, Clone)]
struct Onset {
    /// Offset from the measure start, in ticks.
    offset: u64,
    duration: Duration,
    pitch: u8,
    velocity: u32,
    /// Index of the measure (within the passed slice) this onset lives in.
    measure_index: usize,
}

/// Flatten measures into onsets (lowest note per beat when chords occur,
/// skipping ties and rests). Requires the source tuning to derive pitches.
fn onsets(measures: &[Measure], tuning: Tuning) -> Vec<Onset> {
    let open: std::collections::HashMap<u32, u8> = tuning.iter().copied().collect();
    let mut result = Vec::new();
    for (index, measure) in measures.iter().enumerate() {
        for beat in &measure.beats {
            let mut lowest: Option<(&Note, &Duration)> = None;
            for voice in &beat.voices {
                for note in &voice.notes {
                    if note.tied {
                        continue;
                    }
                    let is_lower = match (&lowest, open.get(&note.string)) {
                        (_, None) => false,
                        (None, Some(_)) => true,
                        (Some((current, _)), Some(&open_pitch)) => {
                            let current_pitch = open.get(&current.string).copied().unwrap_or(0)
                                + current.fret as u8;
                            open_pitch + note.fret as u8 <= current_pitch
                        }
                    };
                    if is_lower {
                        lowest = Some((note, &voice.duration));
                    }
                }
            }
            if let Some((note, duration)) = lowest {
                let pitch = open.get(&note.string).copied().unwrap_or(0) + note.fret as u8;
                result.push(Onset {
                    offset: beat.start_tick.saturating_sub(measure.start_tick),
                    duration: duration.clone(),
                    pitch,
                    velocity: note.velocity,
                    measure_index: index,
                });
            }
        }
    }
    result
}

/// Per-measure root pitch classes: detected from each measure's own notes,
/// falling back to the previous measure, then to the whole passage.
fn measure_roots(measures: &[Measure], all_onsets: &[Onset]) -> Vec<u8> {
    let global_root = detect_scales(
        &all_onsets
            .iter()
            .map(|o| NoteEvent {
                pitch: o.pitch,
                weight: 480,
            })
            .collect::<Vec<_>>(),
    )
    .first()
    .map(|c| c.root_pc)
    .unwrap_or(0);

    let mut roots = Vec::with_capacity(measures.len());
    let mut previous = global_root;
    for index in 0..measures.len() {
        let events: Vec<NoteEvent> = all_onsets
            .iter()
            .filter(|o| o.measure_index == index)
            .map(|o| NoteEvent {
                pitch: o.pitch,
                weight: 480,
            })
            .collect();
        let root = detect_scales(&events)
            .first()
            .map(|c| c.root_pc)
            .unwrap_or(previous);
        roots.push(root);
        previous = root;
    }
    roots
}

fn build_measures(
    template: &[Measure],
    notes: &[(usize, u64, Duration, u8, u32)], // (measure_index, offset, duration, pitch, velocity)
    positions: &[crate::fingering::Position],
) -> Vec<Measure> {
    let mut measures: Vec<Measure> = template
        .iter()
        .map(|m| Measure {
            number: m.number,
            start_tick: 0,
            key_signature: m.key_signature,
            beats: Vec::new(),
        })
        .collect();
    for ((measure_index, offset, duration, _pitch, velocity), position) in
        notes.iter().zip(positions)
    {
        measures[*measure_index].beats.push(Beat {
            start_tick: *offset,
            voices: vec![Voice {
                index: 0,
                duration: duration.clone(),
                is_rest: false,
                notes: vec![Note {
                    string: position.string_number,
                    fret: position.fret,
                    velocity: *velocity,
                    tied: false,
                    effects: NoteEffects::default(),
                }],
            }],
        });
    }
    measures
}

/// Generate a root-following bassline mirroring the source rhythm.
///
/// Returns the measures (numbered like the source) plus a description of
/// the harmonic choices, or an error string when generation is impossible.
pub fn generate_bassline(
    source: &[Measure],
    source_tuning: Tuning,
    bass_tuning: Tuning,
    bass_max_fret: u32,
) -> Result<(Vec<Measure>, String), String> {
    let all_onsets = onsets(source, source_tuning);
    if all_onsets.is_empty() {
        return Err("the source passage contains no notes to follow".into());
    }
    let roots = measure_roots(source, &all_onsets);

    // Lowest playable pitch per pitch class within the bass range.
    let bass_low = bass_tuning.iter().map(|&(_, p)| p).min().unwrap_or(28);
    let to_bass_pitch = |pc: u8| -> u8 {
        let mut pitch = pc % 12;
        while pitch < bass_low {
            pitch += 12;
        }
        pitch
    };

    let mut notes: Vec<(usize, u64, Duration, u8, u32)> = Vec::new();
    for (i, onset) in all_onsets.iter().enumerate() {
        let root = roots[onset.measure_index];
        let mut pitch = to_bass_pitch(root);
        // Chromatic approach into a new root on the last onset of a measure.
        let is_last_of_measure = all_onsets
            .get(i + 1)
            .map(|next| next.measure_index != onset.measure_index)
            .unwrap_or(false);
        if is_last_of_measure {
            if let Some(next_root) = roots.get(onset.measure_index + 1) {
                if *next_root != root {
                    let target = to_bass_pitch(*next_root) as i16;
                    pitch = if target > pitch as i16 {
                        (target - 1) as u8
                    } else {
                        (target + 1) as u8
                    };
                }
            }
        }
        notes.push((
            onset.measure_index,
            onset.offset,
            onset.duration.clone(),
            pitch,
            onset.velocity,
        ));
    }

    let pitches: Vec<u8> = notes.iter().map(|n| n.3).collect();
    let fingering =
        optimize_monophonic(&pitches, bass_tuning, bass_max_fret, &CostModel::default()).map_err(
            |bad| {
                format!(
                    "{} generated note(s) not playable on the bass tuning",
                    bad.len()
                )
            },
        )?;

    let root_names: Vec<String> = roots
        .iter()
        .map(|&pc| crate::pitch::pitch_class_name(pc).to_string())
        .collect();
    let description = format!(
        "root-following bass, {} notes mirroring the source rhythm; roots per measure: {}; \
         chromatic approach notes into root changes",
        notes.len(),
        root_names.join(" ")
    );
    Ok((build_measures(source, &notes, &fingering.path), description))
}

/// Generate a diatonic harmony line above a monophonic source.
/// `interval` is "third" (default) or "sixth".
pub fn generate_harmony(
    source: &[Measure],
    tuning: Tuning,
    max_fret: u32,
    interval: &str,
) -> Result<(Vec<Measure>, String), String> {
    let all_onsets = onsets(source, tuning);
    if all_onsets.is_empty() {
        return Err("the source passage contains no notes to harmonize".into());
    }
    let events: Vec<NoteEvent> = all_onsets
        .iter()
        .map(|o| NoteEvent {
            pitch: o.pitch,
            weight: 480,
        })
        .collect();
    let best = detect_scales(&events)
        .into_iter()
        .next()
        .ok_or_else(|| "could not detect a scale to harmonize in".to_string())?;
    let scale = scale_pitch_classes(best.root_pc, &best.scale)
        .ok_or_else(|| format!("unknown scale: {}", best.scale))?;

    // Candidate offsets, nearest-first, for the requested interval quality.
    let offsets: &[u8] = match interval {
        "sixth" => &[9, 8, 10],
        _ => &[4, 3, 5],
    };
    let in_scale = |pitch: u8| scale.contains(&(pitch % 12));

    let mut notes: Vec<(usize, u64, Duration, u8, u32)> = Vec::new();
    for onset in &all_onsets {
        let harmony_pitch = offsets
            .iter()
            .map(|&o| onset.pitch.saturating_add(o))
            .find(|&p| in_scale(p))
            .unwrap_or(onset.pitch.saturating_add(offsets[0]));
        notes.push((
            onset.measure_index,
            onset.offset,
            onset.duration.clone(),
            harmony_pitch,
            onset.velocity.saturating_sub(5), // sit slightly under the lead
        ));
    }

    let pitches: Vec<u8> = notes.iter().map(|n| n.3).collect();
    let fingering = optimize_monophonic(&pitches, tuning, max_fret, &CostModel::default())
        .map_err(|bad| format!("{} harmony note(s) not playable on this tuning", bad.len()))?;

    let description = format!(
        "diatonic {interval}s above the lead in {} {} ({} notes)",
        best.root,
        best.scale,
        notes.len()
    );
    Ok((build_measures(source, &notes, &fingering.path), description))
}

// Rests are intentionally omitted from generated measures: the bridge's
// autoCompleteSilences fills every gap, so onsets are all we need.
#[allow(dead_code)]
fn _doc_anchor(_: Tuplet) {}

#[cfg(test)]
mod tests {
    use super::*;

    const STANDARD: &[(u32, u8)] = &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];
    const BASS: &[(u32, u8)] = &[(1, 43), (2, 38), (3, 33), (4, 28)];

    fn eighth() -> Duration {
        Duration {
            value: 8,
            dotted: false,
            double_dotted: false,
            tuplet: Tuplet {
                enters: 1,
                times: 1,
            },
        }
    }

    /// A-minor-ish riff on the low strings, two measures.
    fn source() -> Vec<Measure> {
        let steps: [&[(u32, u32)]; 2] = [
            &[(6, 5), (6, 8), (5, 5), (5, 7)],
            &[(4, 5), (4, 7), (5, 5), (6, 5)],
        ];
        steps
            .iter()
            .enumerate()
            .map(|(i, frets)| Measure {
                number: i as u32 + 1,
                start_tick: 960 * (1 + 4 * i as u64),
                key_signature: 0,
                beats: frets
                    .iter()
                    .enumerate()
                    .map(|(j, &(string, fret))| Beat {
                        start_tick: 960 * (1 + 4 * i as u64) + j as u64 * 480,
                        voices: vec![Voice {
                            index: 0,
                            duration: eighth(),
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
            })
            .collect()
    }

    #[test]
    fn bassline_follows_roots_and_rhythm() {
        let (measures, description) =
            generate_bassline(&source(), STANDARD, BASS, 24).expect("generates");
        assert_eq!(measures.len(), 2);
        // Rhythm mirrored: same onset count per measure as the source.
        assert_eq!(measures[0].beats.len(), 4);
        assert_eq!(measures[1].beats.len(), 4);
        // Every generated note is playable on the bass (string 1-4).
        for measure in &measures {
            for beat in &measure.beats {
                let note = &beat.voices[0].notes[0];
                assert!(
                    (1..=4).contains(&note.string),
                    "bass strings only: {note:?}"
                );
            }
        }
        assert!(description.contains("root"), "{description}");
    }

    #[test]
    fn harmony_stays_in_the_detected_scale() {
        let (measures, description) =
            generate_harmony(&source(), STANDARD, 24, "third").expect("generates");
        let scale = scale_pitch_classes(9, "minor pentatonic").unwrap(); // A pent
                                                                         // Harmony pitches must be diatonic to SOME reasonable A-scale; use
                                                                         // A natural minor as the superset check.
        let a_minor = scale_pitch_classes(9, "natural minor").unwrap();
        let open: std::collections::HashMap<u32, u8> = STANDARD.iter().copied().collect();
        for measure in &measures {
            for beat in &measure.beats {
                let note = &beat.voices[0].notes[0];
                let pitch = open[&note.string] + note.fret as u8;
                assert!(
                    a_minor.contains(&(pitch % 12)) || scale.contains(&(pitch % 12)),
                    "harmony note {pitch} out of scale ({description})"
                );
            }
        }
    }

    #[test]
    fn empty_source_is_rejected() {
        let empty = vec![Measure {
            number: 1,
            start_tick: 960,
            key_signature: 0,
            beats: vec![],
        }];
        assert!(generate_bassline(&empty, STANDARD, BASS, 24).is_err());
        assert!(generate_harmony(&empty, STANDARD, 24, "third").is_err());
    }
}
