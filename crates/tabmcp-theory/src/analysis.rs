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
    ("lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
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
