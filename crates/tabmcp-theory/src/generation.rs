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
/// Per-measure lengths in ticks, derived from consecutive start ticks
/// (last measure assumes the previous length; lone measures assume 4/4).
fn measure_lengths(measures: &[Measure]) -> Vec<u64> {
    let mut lengths = Vec::with_capacity(measures.len());
    for i in 0..measures.len() {
        let length = if i + 1 < measures.len() {
            measures[i + 1]
                .start_tick
                .saturating_sub(measures[i].start_tick)
        } else {
            lengths.last().copied().unwrap_or(0)
        };
        lengths.push(if length > 0 { length } else { 3840 });
    }
    lengths
}

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
        // Field finding (modal-metal test): histogram-based detection misreads
        // roots on b2-heavy material. Bass players follow the riff's low
        // anchor — prefer the measure's lowest pitch class when it opens the
        // measure or recurs.
        let measure_events: Vec<&Onset> = all_onsets
            .iter()
            .filter(|o| o.measure_index == index)
            .collect();
        let low_anchor = measure_events
            .iter()
            .map(|o| o.pitch)
            .min()
            .map(|low| low % 12);
        let anchor_is_strong = match low_anchor {
            Some(anchor_pc) => {
                let hits = measure_events
                    .iter()
                    .filter(|o| o.pitch % 12 == anchor_pc)
                    .count();
                let opens = measure_events
                    .first()
                    .map(|o| o.pitch % 12 == anchor_pc)
                    .unwrap_or(false);
                opens || hits * 4 >= measure_events.len()
            }
            None => false,
        };
        let root = if anchor_is_strong {
            low_anchor.unwrap_or(previous)
        } else {
            detect_scales(&events)
                .first()
                .map(|c| c.root_pc)
                .unwrap_or(previous)
        };
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
    let measure_len = measure_lengths(source);

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
        let len = measure_len
            .get(onset.measure_index)
            .copied()
            .unwrap_or(3840);
        if root_unchanged && (len / 2..len * 3 / 4).contains(&onset.offset) {
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
pub const DRUM_HIHAT_OPEN: u32 = 46;
pub const DRUM_CRASH: u32 = 49;
pub const DRUM_RIDE: u32 = 51;

/// One templated hit: (offset ticks, drum key, velocity).
type Hit = (u64, u32, u32);

/// A named groove template for one 4/4 measure (960 ticks per quarter).
/// `follow_accents`: whether kicks additionally double the source's
/// low-register onsets (the original "rock" behavior).
struct GrooveTemplate {
    name: &'static str,
    hits: &'static [Hit],
    follow_accents: bool,
}

const E: u64 = 480; // eighth
const S: u64 = 240; // sixteenth

const GROOVES: &[GrooveTemplate] = &[
    GrooveTemplate {
        name: "rock",
        hits: &[
            (0, DRUM_KICK, 100),
            (2 * E, DRUM_SNARE, 95),
            (4 * E, DRUM_KICK, 100),
            (6 * E, DRUM_SNARE, 95),
            (0, DRUM_HIHAT_CLOSED, 80),
            (E, DRUM_HIHAT_CLOSED, 70),
            (2 * E, DRUM_HIHAT_CLOSED, 80),
            (3 * E, DRUM_HIHAT_CLOSED, 70),
            (4 * E, DRUM_HIHAT_CLOSED, 80),
            (5 * E, DRUM_HIHAT_CLOSED, 70),
            (6 * E, DRUM_HIHAT_CLOSED, 80),
            (7 * E, DRUM_HIHAT_CLOSED, 70),
        ],
        follow_accents: true,
    },
    GrooveTemplate {
        name: "metal-gallop",
        // Kick gallop (eighth + two sixteenths) per beat, snare backbeats,
        // ride carrying the pulse.
        hits: &[
            (0, DRUM_KICK, 105),
            (2 * S, DRUM_KICK, 90),
            (3 * S, DRUM_KICK, 90),
            (2 * E, DRUM_SNARE, 100),
            (4 * E, DRUM_KICK, 105),
            (4 * E + 2 * S, DRUM_KICK, 90),
            (4 * E + 3 * S, DRUM_KICK, 90),
            (6 * E, DRUM_SNARE, 100),
            (0, DRUM_RIDE, 85),
            (E, DRUM_RIDE, 70),
            (2 * E, DRUM_RIDE, 85),
            (3 * E, DRUM_RIDE, 70),
            (4 * E, DRUM_RIDE, 85),
            (5 * E, DRUM_RIDE, 70),
            (6 * E, DRUM_RIDE, 85),
            (7 * E, DRUM_RIDE, 70),
        ],
        follow_accents: false,
    },
    GrooveTemplate {
        name: "punk",
        // Driving: kick on every downbeat eighth, snare 2 and 4, loud hats.
        hits: &[
            (0, DRUM_KICK, 105),
            (E, DRUM_KICK, 95),
            (2 * E, DRUM_SNARE, 105),
            (3 * E, DRUM_KICK, 95),
            (4 * E, DRUM_KICK, 105),
            (5 * E, DRUM_KICK, 95),
            (6 * E, DRUM_SNARE, 105),
            (7 * E, DRUM_KICK, 95),
            (0, DRUM_HIHAT_OPEN, 90),
            (2 * E, DRUM_HIHAT_OPEN, 90),
            (4 * E, DRUM_HIHAT_OPEN, 90),
            (6 * E, DRUM_HIHAT_OPEN, 90),
        ],
        follow_accents: false,
    },
    GrooveTemplate {
        name: "halftime",
        // Heavy: snare only on beat 3, sparse kicks, open feel.
        hits: &[
            (0, DRUM_KICK, 105),
            (3 * E, DRUM_KICK, 90),
            (4 * E, DRUM_SNARE, 105),
            (0, DRUM_HIHAT_CLOSED, 80),
            (E, DRUM_HIHAT_CLOSED, 65),
            (2 * E, DRUM_HIHAT_CLOSED, 80),
            (3 * E, DRUM_HIHAT_CLOSED, 65),
            (4 * E, DRUM_HIHAT_CLOSED, 80),
            (5 * E, DRUM_HIHAT_CLOSED, 65),
            (6 * E, DRUM_HIHAT_CLOSED, 80),
            (7 * E, DRUM_HIHAT_CLOSED, 65),
        ],
        follow_accents: true,
    },
    GrooveTemplate {
        name: "blast",
        // Traditional blast: alternating kick/snare sixteenths, ride on top.
        hits: &[
            (0, DRUM_KICK, 105),
            (S, DRUM_SNARE, 95),
            (2 * S, DRUM_KICK, 100),
            (3 * S, DRUM_SNARE, 95),
            (4 * S, DRUM_KICK, 105),
            (5 * S, DRUM_SNARE, 95),
            (6 * S, DRUM_KICK, 100),
            (7 * S, DRUM_SNARE, 95),
            (8 * S, DRUM_KICK, 105),
            (9 * S, DRUM_SNARE, 95),
            (10 * S, DRUM_KICK, 100),
            (11 * S, DRUM_SNARE, 95),
            (12 * S, DRUM_KICK, 105),
            (13 * S, DRUM_SNARE, 95),
            (14 * S, DRUM_KICK, 100),
            (15 * S, DRUM_SNARE, 95),
            (0, DRUM_RIDE, 80),
            (2 * E, DRUM_RIDE, 80),
            (4 * E, DRUM_RIDE, 80),
            (6 * E, DRUM_RIDE, 80),
        ],
        follow_accents: false,
    },
    GrooveTemplate {
        name: "d-beat",
        // Discharge beat: kick 1 and-of-2 3 and-of-4 feel, snare 2/4, open hats.
        hits: &[
            (0, DRUM_KICK, 105),
            (3 * E, DRUM_KICK, 95),
            (4 * E, DRUM_KICK, 105),
            (7 * E, DRUM_KICK, 95),
            (2 * E, DRUM_SNARE, 105),
            (6 * E, DRUM_SNARE, 105),
            (0, DRUM_HIHAT_OPEN, 90),
            (E, DRUM_HIHAT_CLOSED, 70),
            (2 * E, DRUM_HIHAT_OPEN, 90),
            (3 * E, DRUM_HIHAT_CLOSED, 70),
            (4 * E, DRUM_HIHAT_OPEN, 90),
            (5 * E, DRUM_HIHAT_CLOSED, 70),
            (6 * E, DRUM_HIHAT_OPEN, 90),
            (7 * E, DRUM_HIHAT_CLOSED, 70),
        ],
        follow_accents: false,
    },
];

/// Names of the available drum groove styles.
pub fn drum_styles() -> Vec<&'static str> {
    GROOVES.iter().map(|g| g.name).collect()
}

/// Generate a drum part in a named groove style ("rock", "metal-gallop",
/// "punk", "halftime"), locked to the source where the style follows
/// accents. Percussion track convention: strings tuned to 0, fret = key.
/// Assumes 4/4-ish measures.
pub fn generate_drums(
    source: &[Measure],
    source_tuning: Tuning,
    style: &str,
) -> Result<(Vec<Measure>, String), String> {
    let template = GROOVES.iter().find(|g| g.name == style).ok_or_else(|| {
        format!(
            "unknown drum style '{style}'; available: {}",
            drum_styles().join(", ")
        )
    })?;
    let all_onsets = onsets(source, source_tuning);
    if all_onsets.is_empty() {
        return Err("the source passage contains no notes to follow".into());
    }

    let mut measures = Vec::with_capacity(source.len());
    let mut extra_kicks = 0usize;
    for (index, template_measure) in source.iter().enumerate() {
        // offset -> hits at that slot
        let measure_len = measure_lengths(source).get(index).copied().unwrap_or(3840);
        let mut slots: std::collections::BTreeMap<u64, Vec<(u32, u32)>> =
            std::collections::BTreeMap::new();
        for &(offset, key, velocity) in template.hits {
            // Meter awareness: drop template hits beyond this measure's
            // actual length (templates are written for 4/4).
            if offset < measure_len {
                slots.entry(offset).or_default().push((key, velocity));
            }
        }
        if template.follow_accents {
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
            for onset in &measure_onsets {
                if onset.pitch <= low_threshold {
                    let hits = slots.entry(onset.offset).or_default();
                    if !hits.iter().any(|&(key, _)| key == DRUM_KICK) {
                        hits.push((DRUM_KICK, 100));
                        extra_kicks += 1;
                    }
                }
            }
        }

        let beats = slots
            .into_iter()
            .map(|(offset, hits)| Beat {
                start_tick: offset,
                voices: vec![Voice {
                    index: 0,
                    duration: Duration {
                        // Off-eighth slots (gallop sixteenths) get sixteenths.
                        value: if offset % 480 == 0 { 8 } else { 16 },
                        dotted: false,
                        double_dotted: false,
                        tuplet: Tuplet {
                            enters: 1,
                            times: 1,
                        },
                    },
                    is_rest: false,
                    notes: hits
                        .into_iter()
                        .map(|(key, velocity)| Note {
                            string: match key {
                                DRUM_KICK => 6,
                                DRUM_SNARE => 4,
                                DRUM_CRASH => 2,
                                DRUM_RIDE => 3,
                                _ => 1,
                            },
                            fret: key,
                            velocity,
                            tied: false,
                            effects: NoteEffects::default(),
                        })
                        .collect(),
                }],
            })
            .collect();
        measures.push(Measure {
            number: template_measure.number,
            start_tick: 0,
            key_signature: template_measure.key_signature,
            beats,
        });
    }
    let description = format!(
        "'{}' groove{}",
        template.name,
        if template.follow_accents {
            format!(" with {extra_kicks} kick(s) doubling the source's low-register accents")
        } else {
            String::new()
        }
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
        let (measures, description) =
            generate_drums(&source(), STANDARD, "rock").expect("generates");
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
        assert!(description.contains("'rock' groove"), "{description}");

        // Style templates: gallop has sixteenth kicks, punk has open hats,
        // and unknown styles fail with the available list.
        let (gallop, _) = generate_drums(&source(), STANDARD, "metal-gallop").expect("gallop");
        assert!(
            gallop[0].beats.iter().any(|b| b.start_tick % 480 != 0
                && b.voices[0].notes.iter().any(|n| n.fret == DRUM_KICK)),
            "gallop must place kicks on sixteenth offsets"
        );
        let (punk, _) = generate_drums(&source(), STANDARD, "punk").expect("punk");
        assert!(punk[0]
            .beats
            .iter()
            .any(|b| b.voices[0].notes.iter().any(|n| n.fret == DRUM_HIHAT_OPEN)));
    }

    #[test]
    fn drums_respect_odd_meter_lengths() {
        // Measure 1 = 4/4 (3840 ticks), measure 2 = 7/8 (3360 ticks) —
        // lengths derived from consecutive startTicks.
        let mut m1 = source().remove(0);
        m1.start_tick = 960;
        let mut m2 = source().remove(0);
        m2.number = 2;
        m2.start_tick = 960 + 3840;
        for (j, beat) in m2.beats.iter_mut().enumerate() {
            beat.start_tick = m2.start_tick + j as u64 * 480;
        }
        let mut m3 = source().remove(0);
        m3.number = 3;
        m3.start_tick = 960 + 3840 + 3360; // makes measure 2 read as 7/8
        for (j, beat) in m3.beats.iter_mut().enumerate() {
            beat.start_tick = m3.start_tick + j as u64 * 480;
        }
        let (measures, _) = generate_drums(&[m1, m2, m3], STANDARD, "rock").expect("generates");
        for beat in &measures[1].beats {
            assert!(
                beat.start_tick < 3360,
                "7/8 measure must not receive hits at 4/4 offsets: {}",
                beat.start_tick
            );
        }
        // The 4/4 measure still gets its full backbeat.
        assert!(measures[0].beats.iter().any(|b| b.start_tick == 2880));
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
        assert!(generate_drums(&empty, STANDARD, "rock").is_err());
        assert!(generate_drums(&source(), STANDARD, "nope").is_err());
        assert!(generate_harmony(&empty, STANDARD, 24, "third").is_err());
    }
}
