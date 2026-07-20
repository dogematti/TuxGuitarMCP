//! Scale/key detection and interval analysis over a sequence of note events.
//!
//! Input is deliberately simple: pitches with duration weights, in playing
//! order. The caller (the MCP server) derives pitches from string+fret and
//! the track tuning.

use crate::pitch::{note_name, pitch_class_name};

/// One sounded note, in playing order.
#[derive(Debug, Clone, Copy)]
pub struct NoteEvent {
    /// MIDI pitch.
    pub pitch: u8,
    /// Weight for histograms — duration in ticks works well.
    pub weight: u64,
}

/// A scale candidate, ranked by confidence in [0, 1].
#[derive(Debug, Clone)]
pub struct ScaleCandidate {
    /// Root pitch class, 0 = C.
    pub root_pc: u8,
    /// Root name, e.g. "A".
    pub root: String,
    /// Scale name, e.g. "minor pentatonic".
    pub scale: String,
    pub confidence: f64,
    /// Pitch classes used by the passage that are inside the scale.
    pub covered: usize,
    /// Pitch classes used by the passage that fall outside the scale.
    pub outside: usize,
}

/// (name, semitone offsets from root)
const SCALES: &[(&str, &[u8])] = &[
    ("major", &[0, 2, 4, 5, 7, 9, 11]),
    ("natural minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("harmonic minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("melodic minor", &[0, 2, 3, 5, 7, 9, 11]),
    ("major pentatonic", &[0, 2, 4, 7, 9]),
    ("minor pentatonic", &[0, 3, 5, 7, 10]),
    ("blues", &[0, 3, 5, 6, 7, 10]),
    ("dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("phrygian dominant", &[0, 1, 4, 5, 7, 8, 10]),
    ("lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("locrian", &[0, 1, 3, 5, 6, 8, 10]),
    ("hungarian minor", &[0, 2, 3, 6, 7, 8, 11]),
    ("double harmonic", &[0, 1, 4, 5, 7, 8, 11]),
    ("half-whole diminished", &[0, 1, 3, 4, 6, 7, 9, 10]),
    ("whole tone", &[0, 2, 4, 6, 8, 10]),
    ("altered", &[0, 1, 3, 4, 6, 8, 10]),
    ("lydian dominant", &[0, 2, 4, 6, 7, 9, 10]),
    // Melodic-minor modes
    ("dorian b2", &[0, 1, 3, 5, 7, 9, 10]),
    ("lydian augmented", &[0, 2, 4, 6, 8, 9, 11]),
    ("mixolydian b6", &[0, 2, 4, 5, 7, 8, 10]),
    ("locrian natural 2", &[0, 2, 3, 5, 6, 8, 10]),
    // Harmonic-minor modes
    ("locrian natural 6", &[0, 1, 3, 5, 6, 9, 10]),
    ("ionian #5", &[0, 2, 4, 5, 8, 9, 11]),
    ("dorian #4", &[0, 2, 3, 6, 7, 9, 10]),
    ("lydian #2", &[0, 3, 4, 6, 7, 9, 11]),
    ("super locrian bb7", &[0, 1, 3, 4, 6, 8, 9]),
    // Exotic & world
    ("harmonic major", &[0, 2, 4, 5, 7, 8, 11]),
    ("hungarian major", &[0, 3, 4, 6, 7, 9, 10]),
    ("neapolitan minor", &[0, 1, 3, 5, 7, 8, 11]),
    ("neapolitan major", &[0, 1, 3, 5, 7, 9, 11]),
    ("persian", &[0, 1, 4, 5, 6, 8, 11]),
    ("enigmatic", &[0, 1, 4, 6, 8, 10, 11]),
    ("major blues", &[0, 2, 3, 4, 7, 9]),
    // Japanese pentatonics
    ("hirajoshi", &[0, 2, 3, 7, 8]),
    ("in sen", &[0, 1, 5, 7, 10]),
    ("iwato", &[0, 1, 5, 6, 10]),
    ("yo", &[0, 2, 5, 7, 9]),
    ("egyptian", &[0, 2, 5, 7, 10]),
    // Jazz & symmetric
    ("bebop dominant", &[0, 2, 4, 5, 7, 9, 10, 11]),
    ("bebop major", &[0, 2, 4, 5, 7, 8, 9, 11]),
    ("whole-half diminished", &[0, 2, 3, 5, 6, 8, 9, 11]),
    ("augmented scale", &[0, 3, 4, 7, 8, 11]),
    ("prometheus", &[0, 2, 4, 6, 9, 10]),
];

/// Duration-weighted pitch-class histogram, normalized to sum 1.
pub fn pitch_class_histogram(events: &[NoteEvent]) -> [f64; 12] {
    let mut histogram = [0f64; 12];
    let mut total = 0f64;
    for event in events {
        let weight = event.weight.max(1) as f64;
        histogram[(event.pitch % 12) as usize] += weight;
        total += weight;
    }
    if total > 0.0 {
        for value in &mut histogram {
            *value /= total;
        }
    }
    histogram
}

/// Rank scale candidates for a passage. Empty input returns no candidates.
pub fn detect_scales(events: &[NoteEvent]) -> Vec<ScaleCandidate> {
    if events.is_empty() {
        return Vec::new();
    }
    let histogram = pitch_class_histogram(events);
    let used: Vec<usize> = (0..12).filter(|&pc| histogram[pc] > 0.0).collect();
    let first_pc = (events[0].pitch % 12) as usize;
    let last_pc = (events[events.len() - 1].pitch % 12) as usize;

    // Raw score per (root, scale): coverage minus wrong notes, plus evidence
    // that the root is actually emphasized (weight, final note, opening
    // note), minus a penalty for scale degrees the passage never touches
    // (prefers the tighter scale when several contain the same notes —
    // this is what separates A minor pentatonic from C major).
    let mut scored: Vec<(f64, ScaleCandidate)> = Vec::new();
    for root_pc in 0..12u8 {
        for (scale_name, offsets) in SCALES {
            let member = |pc: usize| offsets.contains(&(((pc + 12 - root_pc as usize) % 12) as u8));

            let mut in_weight = 0f64;
            let mut out_weight = 0f64;
            let mut covered = 0usize;
            let mut outside = 0usize;
            for &pc in &used {
                if member(pc) {
                    in_weight += histogram[pc];
                    covered += 1;
                } else {
                    out_weight += histogram[pc];
                    outside += 1;
                }
            }

            let unused_degrees = offsets.len() - covered.min(offsets.len());
            let score = in_weight - 1.5 * out_weight
                + 0.5 * histogram[root_pc as usize]
                + if last_pc == root_pc as usize {
                    0.2
                } else {
                    0.0
                }
                + if first_pc == root_pc as usize {
                    0.1
                } else {
                    0.0
                }
                - 0.05 * unused_degrees as f64;

            if score > 0.0 {
                scored.push((
                    score,
                    ScaleCandidate {
                        root_pc,
                        root: pitch_class_name(root_pc).to_string(),
                        scale: scale_name.to_string(),
                        confidence: 0.0, // filled below
                        covered,
                        outside,
                    },
                ));
            }
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.truncate(5);

    // Confidence = softmax over the shortlist, so near-ties read as genuine
    // ambiguity and a clear winner reads as high confidence.
    const TEMPERATURE: f64 = 0.2;
    let total: f64 = scored.iter().map(|(s, _)| (s / TEMPERATURE).exp()).sum();
    scored
        .into_iter()
        .map(|(score, mut candidate)| {
            candidate.confidence = (score / TEMPERATURE).exp() / total;
            candidate
        })
        .collect()
}

/// Most plausible tonal center (pitch class name), if any notes are present.
pub fn tonal_center(events: &[NoteEvent]) -> Option<String> {
    detect_scales(events).first().map(|c| c.root.clone())
}

/// (suffix, semitone template) — ordered so richer matches never shadow.
const CHORD_TEMPLATES: &[(&str, &[u8])] = &[
    ("maj7", &[0, 4, 7, 11]),
    ("7", &[0, 4, 7, 10]),
    ("m7", &[0, 3, 7, 10]),
    ("m7b5", &[0, 3, 6, 10]),
    ("", &[0, 4, 7]),
    ("m", &[0, 3, 7]),
    ("dim", &[0, 3, 6]),
    ("aug", &[0, 4, 8]),
    ("sus2", &[0, 2, 7]),
    ("sus4", &[0, 5, 7]),
    ("5", &[0, 7]),
    ("dim7", &[0, 3, 6, 9]),
];

/// Name a chord from its pitch classes (exact template match), e.g.
/// [4, 8, 11] -> "E", [9, 0, 4] -> "Am", [4, 11] -> "E5".
pub fn chord_name(pitch_classes: &[u8]) -> Option<String> {
    let set: std::collections::BTreeSet<u8> = pitch_classes.iter().map(|p| p % 12).collect();
    if set.len() < 2 {
        return None;
    }
    for &root in &set {
        for (suffix, template) in CHORD_TEMPLATES {
            let candidate: std::collections::BTreeSet<u8> =
                template.iter().map(|o| (root + o) % 12).collect();
            if candidate == set {
                return Some(format!("{}{}", pitch_class_name(root), suffix));
            }
        }
    }
    None
}

/// Pitch classes of a named scale at a root (names as in `detect_scales`).
pub fn scale_pitch_classes(root_pc: u8, scale: &str) -> Option<Vec<u8>> {
    SCALES
        .iter()
        .find(|(name, _)| *name == scale)
        .map(|(_, offsets)| offsets.iter().map(|o| (root_pc + o) % 12).collect())
}

const INTERVAL_NAMES: [&str; 13] = [
    "unison",
    "minor 2nd",
    "major 2nd",
    "minor 3rd",
    "major 3rd",
    "perfect 4th",
    "tritone",
    "perfect 5th",
    "minor 6th",
    "major 6th",
    "minor 7th",
    "major 7th",
    "octave",
];

/// Name of the interval spanning `semitones` (compound intervals reduced).
pub fn interval_name(semitones: i32) -> String {
    let magnitude = semitones.unsigned_abs() as usize;
    let reduced = if magnitude > 12 {
        magnitude % 12
    } else {
        magnitude
    };
    let base = INTERVAL_NAMES[reduced];
    match semitones.signum() {
        1 => format!("{base} up"),
        -1 => format!("{base} down"),
        _ => base.to_string(),
    }
}

/// Successive melodic intervals of a passage.
pub fn melodic_intervals(events: &[NoteEvent]) -> Vec<String> {
    events
        .windows(2)
        .map(|pair| interval_name(pair[1].pitch as i32 - pair[0].pitch as i32))
        .collect()
}

/// A note that cannot be transposed on its current string.
#[derive(Debug, Clone)]
pub struct TransposeProblem {
    pub measure: u32,
    pub string: u32,
    pub old_fret: u32,
    pub target_fret: i64,
}

/// Transpose measures in place by `semitones`, keeping every note on its
/// current string (re-fretting). Returns the notes that would fall off the
/// fretboard; if any are returned, the input was NOT modified.
pub fn transpose_measures(
    measures: &mut [tabmcp_model::Measure],
    semitones: i32,
    max_fret: u32,
) -> Vec<TransposeProblem> {
    let mut problems = Vec::new();
    for measure in measures.iter() {
        for beat in &measure.beats {
            for voice in &beat.voices {
                for note in &voice.notes {
                    let target = note.fret as i64 + semitones as i64;
                    if target < 0 || target > max_fret as i64 {
                        problems.push(TransposeProblem {
                            measure: measure.number,
                            string: note.string,
                            old_fret: note.fret,
                            target_fret: target,
                        });
                    }
                }
            }
        }
    }
    if !problems.is_empty() {
        return problems;
    }
    for measure in measures.iter_mut() {
        for beat in &mut measure.beats {
            for voice in &mut beat.voices {
                for note in &mut voice.notes {
                    note.fret = (note.fret as i64 + semitones as i64) as u32;
                }
            }
        }
    }
    problems
}

/// Human-readable summary of a passage: notes, span, intervals, likely scale.
pub fn explain(events: &[NoteEvent]) -> String {
    if events.is_empty() {
        return "The selection contains no notes (only rests).".to_string();
    }
    let names: Vec<String> = events.iter().map(|e| note_name(e.pitch)).collect();
    let lowest = events.iter().map(|e| e.pitch).min().unwrap_or(0);
    let highest = events.iter().map(|e| e.pitch).max().unwrap_or(0);
    let candidates = detect_scales(events);

    let mut out = String::new();
    out.push_str(&format!(
        "Notes ({} total): {}\n",
        names.len(),
        names.join(" ")
    ));
    out.push_str(&format!(
        "Range: {} to {} ({})\n",
        note_name(lowest),
        note_name(highest),
        interval_name((highest as i32) - (lowest as i32)),
    ));

    let intervals = melodic_intervals(events);
    if !intervals.is_empty() {
        out.push_str(&format!("Melodic motion: {}\n", intervals.join(", ")));
    }

    match candidates.first() {
        Some(best) => {
            out.push_str(&format!(
                "Most likely scale: {} {} (confidence {:.0}%)\n",
                best.root,
                best.scale,
                best.confidence * 100.0
            ));
            let alternatives: Vec<String> = candidates
                .iter()
                .skip(1)
                .take(2)
                .map(|c| format!("{} {} ({:.0}%)", c.root, c.scale, c.confidence * 100.0))
                .collect();
            if !alternatives.is_empty() {
                out.push_str(&format!("Alternatives: {}\n", alternatives.join(", ")));
            }
            out.push_str(&format!("Likely tonal center: {}", best.root));
        }
        None => out.push_str("Not enough material to suggest a scale."),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(pitches: &[u8]) -> Vec<NoteEvent> {
        pitches
            .iter()
            .map(|&pitch| NoteEvent { pitch, weight: 480 })
            .collect()
    }

    #[test]
    fn detects_a_minor_pentatonic_riff() {
        // Classic box-1 run: A C D E G A ... ending on A.
        let riff = events(&[57, 60, 62, 64, 67, 69, 67, 64, 62, 60, 57]);
        let best = &detect_scales(&riff)[0];
        assert_eq!(
            (best.root.as_str(), best.scale.as_str()),
            ("A", "minor pentatonic"),
            "root emphasis must beat the relative major"
        );
        assert_eq!(best.outside, 0);
    }

    #[test]
    fn detects_c_major_scale() {
        let scale_run = events(&[60, 62, 64, 65, 67, 69, 71, 72, 71, 69, 67, 65, 64, 62, 60]);
        let best = &detect_scales(&scale_run)[0];
        assert_eq!(best.root, "C");
        assert_eq!(best.scale, "major");
        assert_eq!(tonal_center(&scale_run).as_deref(), Some("C"));
    }

    #[test]
    fn interval_names_are_directional() {
        assert_eq!(interval_name(7), "perfect 5th up");
        assert_eq!(interval_name(-3), "minor 3rd down");
        assert_eq!(interval_name(0), "unison");
        assert_eq!(interval_name(12), "octave up");
    }

    #[test]
    fn explain_mentions_notes_and_scale() {
        let riff = events(&[57, 60, 62, 64, 67, 69]);
        let text = explain(&riff);
        assert!(text.contains("A3"), "should list note names: {text}");
        assert!(text.contains("Most likely scale"), "{text}");
    }

    #[test]
    fn empty_selection_is_handled() {
        assert!(explain(&[]).contains("no notes"));
        assert!(detect_scales(&[]).is_empty());
    }
}

#[cfg(test)]
mod chord_name_tests {
    use super::chord_name;

    #[test]
    fn names_common_chords() {
        assert_eq!(chord_name(&[4, 8, 11]).as_deref(), Some("E"));
        assert_eq!(chord_name(&[9, 0, 4]).as_deref(), Some("Am"));
        assert_eq!(chord_name(&[4, 11]).as_deref(), Some("E5"));
        assert_eq!(chord_name(&[7, 11, 2, 5]).as_deref(), Some("G7"));
        assert_eq!(chord_name(&[5]).as_deref(), None);
        assert_eq!(chord_name(&[0, 1, 2]).as_deref(), None);
    }
}

#[cfg(test)]
mod metal_scale_tests {
    use super::*;

    #[test]
    fn detects_e_phrygian_dominant() {
        // E F G# A B C D — the flamenco/metal staple, root-anchored on E.
        let riff: Vec<NoteEvent> = [40u8, 41, 44, 45, 47, 48, 50, 48, 47, 45, 44, 41, 40]
            .iter()
            .map(|&pitch| NoteEvent { pitch, weight: 480 })
            .collect();
        let best = &detect_scales(&riff)[0];
        assert_eq!(
            (best.root.as_str(), best.scale.as_str()),
            ("E", "phrygian dominant"),
            "got {} {}",
            best.root,
            best.scale
        );
        // Harmony generation can voice in it too.
        assert!(scale_pitch_classes(4, "phrygian dominant").is_some());
    }
}
