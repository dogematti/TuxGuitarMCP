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

    // Lowest pitch we allow the bass to sit on. The tuning may reach E1
    // (MIDI 28), but common GM soundfonts (including TuxGuitar's own
    // MagicSFver2) do not voice that bottom octave — a bass written there
    // renders silent. Keep roots at E2 (40) or above.
    const BASS_FLOOR: u8 = 40;
    let bass_low = bass_tuning
        .iter()
        .map(|&(_, p)| p)
        .min()
        .unwrap_or(28)
        .max(BASS_FLOOR);
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
        // Static harmony must not become a one-note drone: walk to the
        // fifth on the second half of the measure when the root persists.
        let root_unchanged = roots
            .get(onset.measure_index.wrapping_sub(1))
            .map(|&prev| prev == root && onset.measure_index > 0)
            .unwrap_or(false);
        if root_unchanged && (1920..2880).contains(&onset.offset) {
            pitch = to_bass_pitch((root + 7) % 12);
        }
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

/// General-MIDI drum keys used by the drum generator.
pub const DRUM_KICK: u32 = 36;
pub const DRUM_SNARE: u32 = 38;
pub const DRUM_HIHAT_CLOSED: u32 = 42;

/// Generate a basic rock/metal drum part locked to the source's accents:
/// closed hi-hat eighths, snare backbeats (beats 2 and 4), kick doubling
/// the source's low-register onsets. Written for a percussion track whose
/// strings are all tuned to 0, so fret == drum key.
/// Assumes 4/4-ish measures (the eighth grid spans 8 slots); odd meters
/// get hi-hats only on actual source onsets.
pub fn generate_drums(
    source: &[Measure],
    source_tuning: Tuning,
) -> Result<(Vec<Measure>, String), String> {
    let all_onsets = onsets(source, source_tuning);
    if all_onsets.is_empty() {
        return Err("the source passage contains no notes to follow".into());
    }
    let eighth = Duration {
        value: 8,
        dotted: false,
        double_dotted: false,
        tuplet: Tuplet {
            enters: 1,
            times: 1,
        },
    };

    let mut measures = Vec::with_capacity(source.len());
    let mut kicks = 0usize;
    for (index, template) in source.iter().enumerate() {
        let measure_onsets: Vec<&Onset> = all_onsets
            .iter()
            .filter(|o| o.measure_index == index)
            .collect();
        let low_threshold = measure_onsets
            .iter()
            .map(|o| o.pitch)
            .min()
            .unwrap_or(0)
            .saturating_add(2);

        // offset -> drum keys at that slot
        let mut slots: std::collections::BTreeMap<u64, Vec<u32>> =
            std::collections::BTreeMap::new();
        for slot in 0..8u64 {
            slots.insert(slot * 480, vec![DRUM_HIHAT_CLOSED]);
        }
        for onset in &measure_onsets {
            slots
                .entry(onset.offset)
                .or_insert_with(|| vec![DRUM_HIHAT_CLOSED]);
        }
        for &backbeat in &[960u64, 2880] {
            slots.entry(backbeat).or_default().push(DRUM_SNARE);
        }
        for onset in &measure_onsets {
            if onset.pitch <= low_threshold {
                let keys = slots.entry(onset.offset).or_default();
                if !keys.contains(&DRUM_KICK) {
                    keys.push(DRUM_KICK);
                    kicks += 1;
                }
            }
        }
        // A drummer anchors the downbeat regardless of what the guitar does.
        let downbeat = slots.entry(0).or_default();
        if !downbeat.contains(&DRUM_KICK) {
            downbeat.push(DRUM_KICK);
            kicks += 1;
        }

        let beats = slots
            .into_iter()
            .map(|(offset, keys)| Beat {
                start_tick: offset,
                voices: vec![Voice {
                    index: 0,
                    duration: eighth.clone(),
                    is_rest: false,
                    notes: keys
                        .into_iter()
                        .map(|key| Note {
                            // Distinct strings per instrument; fret = drum key
                            // (percussion strings are tuned to 0).
                            string: match key {
                                DRUM_KICK => 6,
                                DRUM_SNARE => 4,
                                _ => 1,
                            },
                            fret: key,
                            velocity: match key {
                                DRUM_KICK => 100,
                                DRUM_SNARE => 95,
                                _ => 75,
                            },
                            tied: false,
                            effects: NoteEffects::default(),
                        })
                        .collect(),
                }],
            })
            .collect();
        measures.push(Measure {
            number: template.number,
            start_tick: 0,
            key_signature: template.key_signature,
            beats,
        });
    }
    let description = format!(
        "closed hi-hat eighths, snare on beats 2 and 4, {kicks} kick(s) doubling the \
         source's low-register accents"
    );
    Ok((measures, description))
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
    fn bassline_stays_above_the_soundfont_floor_and_moves_on_static_harmony() {
        // Single-root source (all E, full 8-eighth measures) across two
        // measures: the old generator parked every note on E1, which is
        // silent in common soundfonts.
        let source_measures: Vec<Measure> = (1..=2u32)
            .map(|n| {
                let start = 960 * (1 + 4 * (n as u64 - 1));
                Measure {
                    number: n,
                    start_tick: start,
                    key_signature: 0,
                    beats: (0..8u64)
                        .map(|j| Beat {
                            start_tick: start + j * 480,
                            voices: vec![Voice {
                                index: 0,
                                duration: eighth(),
                                is_rest: false,
                                notes: vec![Note {
                                    string: 6,
                                    fret: 0,
                                    velocity: 95,
                                    tied: false,
                                    effects: NoteEffects::default(),
                                }],
                            }],
                        })
                        .collect(),
                }
            })
            .collect();
        let (measures, _) =
            generate_bassline(&source_measures, STANDARD, BASS, 24).expect("generates");
        let open: std::collections::HashMap<u32, u8> = BASS.iter().copied().collect();
        let pitches: Vec<u8> = measures
            .iter()
            .flat_map(|m| &m.beats)
            .flat_map(|b| &b.voices)
            .flat_map(|v| &v.notes)
            .map(|n| open[&n.string] + n.fret as u8)
            .collect();
        assert!(
            pitches.iter().all(|&p| p >= 40),
            "bass must stay at E2+ (soundfont floor): {pitches:?}"
        );
        assert!(
            pitches.iter().any(|&p| p % 12 == 11),
            "static harmony should walk to the fifth (B over E): {pitches:?}"
        );
    }

    #[test]
    fn drums_lock_to_accents_and_backbeats() {
        let (measures, description) = generate_drums(&source(), STANDARD).expect("generates");
        assert_eq!(measures.len(), 2);
        for measure in &measures {
            let mut has_snare_backbeat = false;
            let mut has_kick_on_one = false;
            for beat in &measure.beats {
                for note in &beat.voices[0].notes {
                    if note.fret == DRUM_SNARE
                        && (beat.start_tick == 960 || beat.start_tick == 2880)
                    {
                        has_snare_backbeat = true;
                    }
                    if note.fret == DRUM_KICK && beat.start_tick == 0 {
                        has_kick_on_one = true;
                    }
                }
            }
            assert!(has_snare_backbeat, "snare must hit the backbeat");
            assert!(
                has_kick_on_one,
                "kick must land on beat 1 (lowest source note)"
            );
        }
        assert!(description.contains("hi-hat"), "{description}");
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
