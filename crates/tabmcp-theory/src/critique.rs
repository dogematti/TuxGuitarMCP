//! AI Ear evaluators: the per-track musical-quality metrics that feed the
//! refinement loop (generate -> evaluate -> improve -> repeat). The AI
//! client is the critic; these functions are its measuring instruments.

use std::collections::HashMap;

use tabmcp_model::Measure;

use crate::fingering::Tuning;

struct Onset {
    pitch: u8,
    velocity: u32,
    offset: u64,
    measure_index: usize,
}

fn onsets(measures: &[Measure], tuning: Tuning) -> Vec<Onset> {
    let open: HashMap<u32, u8> = tuning.iter().copied().collect();
    let mut out = Vec::new();
    for (index, measure) in measures.iter().enumerate() {
        for beat in &measure.beats {
            for voice in &beat.voices {
                for note in &voice.notes {
                    if note.tied {
                        continue;
                    }
                    if let Some(&open_pitch) = open.get(&note.string) {
                        out.push(Onset {
                            pitch: open_pitch.saturating_add(note.fret as u8),
                            velocity: note.velocity,
                            offset: beat.start_tick.saturating_sub(measure.start_tick),
                            measure_index: index,
                        });
                    }
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct CritiqueReport {
    /// 0..1: how dominant the most common inter-onset interval is
    /// (1.0 = perfectly regular pulse, low = erratic rhythm).
    pub groove_consistency: f64,
    /// Note-count spread between the busiest and sparsest measure.
    pub density_range: (usize, usize),
    /// 0..1: fraction of the line covered by repeated interval motifs.
    pub motif_repetition: f64,
    /// The most repeated interval pattern, as semitone steps.
    pub top_motif: Vec<i8>,
    pub velocity_mean: f64,
    /// Velocity standard deviation — near 0 means robotic dynamics.
    pub velocity_std: f64,
}

/// Evaluate one track's material.
pub fn critique(measures: &[Measure], tuning: Tuning) -> CritiqueReport {
    let events = onsets(measures, tuning);
    if events.is_empty() {
        return CritiqueReport::default();
    }

    // Groove: histogram of inter-onset intervals within measures.
    let mut interval_counts: HashMap<u64, usize> = HashMap::new();
    let mut total_intervals = 0usize;
    for pair in events.windows(2) {
        if pair[0].measure_index == pair[1].measure_index {
            *interval_counts
                .entry(pair[1].offset.saturating_sub(pair[0].offset))
                .or_default() += 1;
            total_intervals += 1;
        }
    }
    let groove_consistency = interval_counts
        .values()
        .max()
        .map(|&top| top as f64 / total_intervals.max(1) as f64)
        .unwrap_or(1.0);

    // Density per measure.
    let mut per_measure: HashMap<usize, usize> = HashMap::new();
    for event in &events {
        *per_measure.entry(event.measure_index).or_default() += 1;
    }
    for i in 0..measures.len() {
        per_measure.entry(i).or_default();
    }
    let density_range = (
        per_measure.values().copied().min().unwrap_or(0),
        per_measure.values().copied().max().unwrap_or(0),
    );

    // Motifs: repeated interval n-grams (length 3..=5).
    let intervals: Vec<i8> = events
        .windows(2)
        .map(|p| (p[1].pitch as i16 - p[0].pitch as i16).clamp(-127, 127) as i8)
        .collect();
    let mut best_covered = 0usize;
    let mut top_motif: Vec<i8> = Vec::new();
    for len in 3..=5usize {
        if intervals.len() < len * 2 {
            continue;
        }
        let mut counts: HashMap<&[i8], usize> = HashMap::new();
        for window in intervals.windows(len) {
            *counts.entry(window).or_default() += 1;
        }
        if let Some((motif, &count)) = counts.iter().max_by_key(|(_, &c)| c) {
            if count >= 2 {
                let covered = count * len;
                if covered > best_covered {
                    best_covered = covered;
                    top_motif = motif.to_vec();
                }
            }
        }
    }
    let motif_repetition = (best_covered as f64 / intervals.len().max(1) as f64).min(1.0);

    // Dynamics.
    let velocity_mean = events.iter().map(|e| e.velocity as f64).sum::<f64>() / events.len() as f64;
    let velocity_std = (events
        .iter()
        .map(|e| (e.velocity as f64 - velocity_mean).powi(2))
        .sum::<f64>()
        / events.len() as f64)
        .sqrt();

    CritiqueReport {
        groove_consistency,
        density_range,
        motif_repetition,
        top_motif,
        velocity_mean,
        velocity_std,
    }
}

/// Render the critique as issues + observations for the refinement loop.
pub fn describe(report: &CritiqueReport, track_label: &str) -> String {
    let mut out = format!(
        "{track_label}: groove consistency {:.0}%, motif repetition {:.0}%, \
         density {}..{} notes/measure, velocity {:.0}±{:.0}\n",
        report.groove_consistency * 100.0,
        report.motif_repetition * 100.0,
        report.density_range.0,
        report.density_range.1,
        report.velocity_mean,
        report.velocity_std,
    );
    if !report.top_motif.is_empty() {
        out.push_str(&format!(
            "  motif: interval pattern {:?} recurs — develop or vary it deliberately\n",
            report.top_motif
        ));
    }
    if report.groove_consistency < 0.5 {
        out.push_str("  ISSUE: erratic rhythm — no dominant pulse subdivision\n");
    }
    if report.motif_repetition < 0.15 {
        out.push_str("  ISSUE: little repetition — the line may read as wandering\n");
    }
    if report.velocity_std < 2.0 {
        out.push_str("  ISSUE: robotic dynamics — consider tuxguitar_humanize or accents\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tabmcp_model::{Beat, Duration, Note, NoteEffects, Tuplet, Voice};

    const STANDARD: &[(u32, u8)] = &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];

    fn measure(number: u32, steps: &[(u32, u32, u32)]) -> Measure {
        Measure {
            number,
            start_tick: 960 * (1 + 4 * (number as u64 - 1)),
            key_signature: 0,
            beats: steps
                .iter()
                .enumerate()
                .map(|(j, &(string, fret, velocity))| Beat {
                    start_tick: 960 * (1 + 4 * (number as u64 - 1)) + j as u64 * 480,
                    voices: vec![Voice {
                        index: 0,
                        duration: Duration {
                            value: 8,
                            dotted: false,
                            double_dotted: false,
                            tuplet: Tuplet {
                                enters: 1,
                                times: 1,
                            },
                        },
                        is_rest: false,
                        notes: vec![Note {
                            string,
                            fret,
                            velocity,
                            tied: false,
                            effects: NoteEffects::default(),
                        }],
                    }],
                })
                .collect(),
        }
    }

    #[test]
    fn repeated_riff_scores_high_motif_and_groove() {
        // The same 4-note figure twice per measure, two measures, flat velocity.
        let riff = [(6, 0, 95), (6, 3, 95), (6, 5, 95), (5, 2, 95)];
        let steps: Vec<(u32, u32, u32)> = riff.iter().chain(riff.iter()).copied().collect();
        let measures = vec![measure(1, &steps), measure(2, &steps)];
        let report = critique(&measures, STANDARD);
        assert!(
            report.groove_consistency > 0.9,
            "{}",
            report.groove_consistency
        );
        assert!(report.motif_repetition > 0.4, "{}", report.motif_repetition);
        assert!(report.velocity_std < 2.0);
        let text = describe(&report, "T1");
        assert!(text.contains("robotic dynamics"), "{text}");
    }

    #[test]
    fn varied_velocities_clear_the_dynamics_issue() {
        let steps = [(6, 0, 80), (6, 3, 110), (6, 5, 70), (5, 2, 100)];
        let measures = vec![measure(1, &steps)];
        let report = critique(&measures, STANDARD);
        assert!(report.velocity_std > 10.0);
        assert!(!describe(&report, "T1").contains("robotic"));
    }
}
