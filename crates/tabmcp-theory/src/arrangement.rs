//! Virtual ear v1: cross-track arrangement analysis.
//!
//! Claude cannot hear audio in this pipeline, but for symbolic music nearly
//! everything audible is in the score. This module "listens" the way a
//! producer reads a session: vertical dissonance between tracks, register
//! collisions, rhythmic tightness, empty bars, and level balance — and
//! reports it in a form an AI client can act on with the edit tools.

use std::collections::{BTreeMap, BTreeSet};

use tabmcp_model::Measure;

use crate::pitch::note_name;

/// One track's material, as read from the bridge.
pub struct TrackInput {
    pub number: u32,
    pub name: String,
    pub is_percussion: bool,
    pub tuning: Vec<(u32, u8)>,
    pub measures: Vec<Measure>,
}

#[derive(Debug, Clone)]
pub struct TrackReport {
    pub number: u32,
    pub name: String,
    pub note_count: usize,
    pub avg_velocity: f64,
    /// (lowest, highest) sounding pitches, None for percussion/empty.
    pub pitch_range: Option<(u8, u8)>,
    /// Measure numbers with no notes at all.
    pub empty_measures: Vec<u32>,
}

/// A harsh vertical interval between two tracks at one moment.
#[derive(Debug, Clone)]
pub struct Clash {
    pub measure: u32,
    /// Offset within the measure, in ticks.
    pub tick_offset: u64,
    pub track_a: u32,
    pub track_b: u32,
    pub pitch_a: u8,
    pub pitch_b: u8,
    /// Interval class in semitones (1 = minor-2nd family, 6 = tritone).
    pub interval_class: u8,
}

#[derive(Debug, Clone)]
pub struct ArrangementReport {
    pub tracks: Vec<TrackReport>,
    pub clashes: Vec<Clash>,
    /// Fraction (0..1) of melodic-track onsets that coincide with another
    /// track's onset — a crude tightness measure.
    pub onset_alignment: f64,
    /// Pairs of melodic tracks whose pitch ranges overlap by more than an
    /// octave (candidates for register masking): (track_a, track_b, overlap).
    pub register_overlaps: Vec<(u32, u32, u8)>,
}

struct SoundingNote {
    track: u32,
    pitch: u8,
    measure: u32,
    tick_offset: u64,
}

fn events(track: &TrackInput) -> (Vec<SoundingNote>, BTreeSet<u64>, Vec<u32>, usize, f64) {
    let open: std::collections::HashMap<u32, u8> = track.tuning.iter().copied().collect();
    let mut notes = Vec::new();
    let mut onsets = BTreeSet::new();
    let mut empty = Vec::new();
    let mut velocity_sum = 0u64;
    let mut note_total = 0usize;
    for measure in &track.measures {
        let mut measure_has_notes = false;
        for beat in &measure.beats {
            for voice in &beat.voices {
                for note in &voice.notes {
                    if note.tied {
                        continue;
                    }
                    measure_has_notes = true;
                    note_total += 1;
                    velocity_sum += note.velocity as u64;
                    let absolute = beat.start_tick.max(measure.start_tick);
                    onsets.insert(absolute);
                    if !track.is_percussion {
                        if let Some(&open_pitch) = open.get(&note.string) {
                            notes.push(SoundingNote {
                                track: track.number,
                                pitch: open_pitch.saturating_add(note.fret as u8),
                                measure: measure.number,
                                tick_offset: absolute.saturating_sub(measure.start_tick),
                            });
                        }
                    }
                }
            }
        }
        if !measure_has_notes {
            empty.push(measure.number);
        }
    }
    let avg_velocity = if note_total > 0 {
        velocity_sum as f64 / note_total as f64
    } else {
        0.0
    };
    (notes, onsets, empty, note_total, avg_velocity)
}

/// Analyze the vertical/horizontal relationships between tracks.
pub fn analyze_arrangement(tracks: &[TrackInput]) -> ArrangementReport {
    let mut reports = Vec::new();
    let mut all_notes: Vec<SoundingNote> = Vec::new();
    let mut onsets_per_track: Vec<(u32, BTreeSet<u64>, bool)> = Vec::new();

    for track in tracks {
        let (notes, onsets, empty, count, avg_velocity) = events(track);
        let pitch_range = if track.is_percussion || notes.is_empty() {
            None
        } else {
            Some((
                notes.iter().map(|n| n.pitch).min().unwrap_or(0),
                notes.iter().map(|n| n.pitch).max().unwrap_or(0),
            ))
        };
        reports.push(TrackReport {
            number: track.number,
            name: track.name.clone(),
            note_count: count,
            avg_velocity,
            pitch_range,
            empty_measures: empty,
        });
        onsets_per_track.push((track.number, onsets, track.is_percussion));
        all_notes.extend(notes);
    }

    // Vertical dissonance: group simultaneous melodic notes by absolute-ish
    // position (measure, tick offset) and flag semitone/tritone classes
    // between DIFFERENT tracks.
    let mut by_moment: BTreeMap<(u32, u64), Vec<&SoundingNote>> = BTreeMap::new();
    for note in &all_notes {
        by_moment
            .entry((note.measure, note.tick_offset))
            .or_default()
            .push(note);
    }
    let mut clashes = Vec::new();
    for ((measure, tick_offset), moment) in &by_moment {
        for (i, a) in moment.iter().enumerate() {
            for b in moment.iter().skip(i + 1) {
                if a.track == b.track {
                    continue;
                }
                let distance = (a.pitch as i16 - b.pitch as i16).unsigned_abs() as u8;
                let class = distance % 12;
                let class = class.min(12 - class); // interval class 0..6
                if class == 1 || class == 6 {
                    clashes.push(Clash {
                        measure: *measure,
                        tick_offset: *tick_offset,
                        track_a: a.track,
                        track_b: b.track,
                        pitch_a: a.pitch,
                        pitch_b: b.pitch,
                        interval_class: class,
                    });
                }
            }
        }
    }

    // Onset alignment between melodic tracks.
    let melodic: Vec<&(u32, BTreeSet<u64>, bool)> = onsets_per_track
        .iter()
        .filter(|(_, _, perc)| !perc)
        .collect();
    let mut shared = 0usize;
    let mut total = 0usize;
    for (i, (_, onsets, _)) in melodic.iter().enumerate() {
        for tick in onsets.iter() {
            total += 1;
            if melodic
                .iter()
                .enumerate()
                .any(|(j, (_, other, _))| i != j && other.contains(tick))
            {
                shared += 1;
            }
        }
    }
    let onset_alignment = if total > 0 {
        shared as f64 / total as f64
    } else {
        0.0
    };

    // Register overlap between melodic tracks (> an octave of shared range).
    let mut register_overlaps = Vec::new();
    for (i, a) in reports.iter().enumerate() {
        for b in reports.iter().skip(i + 1) {
            if let (Some((low_a, high_a)), Some((low_b, high_b))) = (a.pitch_range, b.pitch_range) {
                let overlap = high_a.min(high_b) as i16 - low_a.max(low_b) as i16;
                if overlap > 12 {
                    register_overlaps.push((a.number, b.number, overlap as u8));
                }
            }
        }
    }

    ArrangementReport {
        tracks: reports,
        clashes,
        onset_alignment,
        register_overlaps,
    }
}

/// Producer's notes: the report as readable text for the AI client.
pub fn describe(report: &ArrangementReport) -> String {
    let mut out = String::new();
    for track in &report.tracks {
        let range = match track.pitch_range {
            Some((low, high)) => format!("{}..{}", note_name(low), note_name(high)),
            None => "-".into(),
        };
        out.push_str(&format!(
            "Track {} \"{}\": {} notes, range {}, avg velocity {:.0}{}\n",
            track.number,
            track.name,
            track.note_count,
            range,
            track.avg_velocity,
            if track.empty_measures.is_empty() {
                String::new()
            } else {
                format!(
                    ", EMPTY in measure(s) {}",
                    track
                        .empty_measures
                        .iter()
                        .map(|m| m.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            },
        ));
    }
    out.push_str(&format!(
        "Rhythmic tightness: {:.0}% of onsets align across tracks\n",
        report.onset_alignment * 100.0
    ));
    for (a, b, overlap) in &report.register_overlaps {
        out.push_str(&format!(
            "Register: tracks {a} and {b} share {overlap} semitones of range — \
             consider separating octaves to avoid masking\n"
        ));
    }
    if report.clashes.is_empty() {
        out.push_str("No harsh cross-track dissonances (minor 2nds / tritones) detected.\n");
    } else {
        out.push_str(&format!(
            "{} harsh cross-track clash(es):\n",
            report.clashes.len()
        ));
        for clash in report.clashes.iter().take(8) {
            out.push_str(&format!(
                "  measure {}, tick {}: track {} {} vs track {} {} ({})\n",
                clash.measure,
                clash.tick_offset,
                clash.track_a,
                note_name(clash.pitch_a),
                clash.track_b,
                note_name(clash.pitch_b),
                if clash.interval_class == 1 {
                    "semitone"
                } else {
                    "tritone"
                },
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tabmcp_model::{Beat, Duration, Note, NoteEffects, Tuplet, Voice};

    const STANDARD: &[(u32, u8)] = &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];

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

    fn track(number: u32, frets: Vec<Vec<(u32, u32)>>) -> TrackInput {
        let measures = frets
            .into_iter()
            .enumerate()
            .map(|(i, steps)| Measure {
                number: i as u32 + 1,
                start_tick: 960 * (1 + 4 * i as u64),
                key_signature: 0,
                beats: steps
                    .into_iter()
                    .enumerate()
                    .map(|(j, (string, fret))| Beat {
                        start_tick: 960 * (1 + 4 * i as u64) + j as u64 * 480,
                        voices: vec![Voice {
                            index: 0,
                            duration: eighth(),
                            is_rest: false,
                            notes: vec![Note {
                                string,
                                fret,
                                velocity: 90,
                                tied: false,
                                effects: NoteEffects::default(),
                            }],
                        }],
                    })
                    .collect(),
            })
            .collect();
        TrackInput {
            number,
            name: format!("T{number}"),
            is_percussion: false,
            tuning: STANDARD.to_vec(),
            measures,
        }
    }

    #[test]
    fn detects_semitone_clash_between_tracks() {
        // Track 1 plays E2 (6,0); track 2 plays F2 (6,1) at the same tick.
        let a = track(1, vec![vec![(6, 0), (6, 0)]]);
        let b = track(2, vec![vec![(6, 1), (6, 5)]]);
        let report = analyze_arrangement(&[a, b]);
        assert_eq!(report.clashes.len(), 1, "exactly the first beat clashes");
        let clash = &report.clashes[0];
        assert_eq!(clash.interval_class, 1);
        assert_eq!(clash.measure, 1);
        assert_eq!(clash.tick_offset, 0);
        assert!(describe(&report).contains("semitone"));
    }

    #[test]
    fn reports_empty_measures_and_alignment() {
        let a = track(1, vec![vec![(6, 0), (6, 3)], vec![]]);
        let b = track(2, vec![vec![(5, 2), (5, 5)], vec![(5, 2)]]);
        let report = analyze_arrangement(&[a, b]);
        let track_a = &report.tracks[0];
        assert_eq!(track_a.empty_measures, vec![2]);
        // Measure 1 onsets align fully between the tracks.
        assert!(report.onset_alignment > 0.5, "{}", report.onset_alignment);
        assert!(describe(&report).contains("EMPTY in measure(s) 2"));
    }

    #[test]
    fn clean_arrangement_reports_no_clashes() {
        // Octaves between tracks: consonant.
        let a = track(1, vec![vec![(6, 0)]]); // E2
        let b = track(2, vec![vec![(4, 2)]]); // E3
        let report = analyze_arrangement(&[a, b]);
        assert!(report.clashes.is_empty());
        assert!(describe(&report).contains("No harsh"));
    }
}
