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
    /// 0..1 weighted share of onsets off the strong beats (0 = metronomic,
    /// on-quarter; 1 = nothing lands on a beat).
    pub syncopation: f64,
    /// Measures that are exact copies of an earlier measure.
    pub literal_repeats: usize,
    /// Measures that develop earlier material (same rhythm, new pitches — or
    /// same interval line, new rhythm).
    pub varied_repeats: usize,
    /// Non-empty measures introducing new material (the first one counts).
    pub fresh_measures: usize,
    /// Per-measure tension 0..1 (density + dynamics + register + dissonance,
    /// normalized within the range). Index 0 = first analyzed measure.
    pub tension: Vec<f64>,
    /// Root changes per measure boundary, 0..1 (0 = one pedal root
    /// throughout, 1 = new bass root every measure).
    pub harmonic_rhythm: f64,
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
    // Field finding: a gallop is bimodal (240-240-480...) yet perfectly
    // consistent. Score the best REPEATING IOI pattern (period 1..=4), with
    // the single-interval share as the period-1 case.
    let iois: Vec<u64> = events
        .windows(2)
        .filter(|p| p[0].measure_index == p[1].measure_index)
        .map(|p| p[1].offset.saturating_sub(p[0].offset))
        .collect();
    let groove_consistency = if iois.is_empty() {
        1.0
    } else {
        (1..=4usize.min(iois.len()))
            .map(|period| {
                let pattern = &iois[..period];
                let matches = iois
                    .iter()
                    .enumerate()
                    .filter(|(i, &v)| v == pattern[i % period])
                    .count();
                matches as f64 / iois.len() as f64
            })
            .fold(0.0f64, f64::max)
    };
    let _ = (&interval_counts, total_intervals); // superseded by periodicity

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

    // Syncopation: weight each onset by how far off the beat it lands.
    // On a quarter (offset % 960 == 0) -> 0, on an offbeat 8th -> 0.5,
    // finer positions -> 1.0.
    let syncopation = events
        .iter()
        .map(|e| {
            if e.offset % 960 == 0 {
                0.0
            } else if e.offset % 480 == 0 {
                0.5
            } else {
                1.0
            }
        })
        .sum::<f64>()
        / events.len() as f64;

    // Motif development: classify each non-empty measure against all earlier
    // ones — literal copy, varied repeat (shared rhythm OR shared interval
    // line), or fresh material.
    #[derive(PartialEq)]
    struct Signature {
        rhythm: Vec<u64>,
        pitches: Vec<u8>,
        intervals: Vec<i16>,
    }
    let signatures: Vec<Option<Signature>> = (0..measures.len())
        .map(|i| {
            let in_measure: Vec<&Onset> =
                events.iter().filter(|e| e.measure_index == i).collect();
            if in_measure.is_empty() {
                return None;
            }
            let pitches: Vec<u8> = in_measure.iter().map(|e| e.pitch).collect();
            Some(Signature {
                rhythm: in_measure.iter().map(|e| e.offset).collect(),
                intervals: pitches
                    .windows(2)
                    .map(|p| p[1] as i16 - p[0] as i16)
                    .collect(),
                pitches,
            })
        })
        .collect();
    let (mut literal_repeats, mut varied_repeats, mut fresh_measures) = (0usize, 0usize, 0usize);
    for (i, sig) in signatures.iter().enumerate() {
        let Some(sig) = sig else { continue };
        let earlier = signatures[..i].iter().flatten();
        let mut best = 0u8; // 0 fresh, 1 varied, 2 literal
        for other in earlier {
            if sig == other {
                best = 2;
                break;
            }
            let shared_rhythm = sig.rhythm == other.rhythm && sig.pitches != other.pitches;
            let shared_line = !sig.intervals.is_empty()
                && sig.intervals == other.intervals
                && sig.rhythm != other.rhythm;
            if shared_rhythm || shared_line {
                best = best.max(1);
            }
        }
        match best {
            2 => literal_repeats += 1,
            1 => varied_repeats += 1,
            _ => fresh_measures += 1,
        }
    }

    // Tension curve: per-measure composite of density, dynamics, register,
    // and dissonant melodic motion (semitones/tritones), normalized so the
    // range's own extremes define 0..1.
    let tension: Vec<f64> = {
        // Two passes: gather raw tuples, then normalize each component.
        let tuples: Vec<(f64, f64, f64, f64)> = {
            let mut t = Vec::with_capacity(measures.len());
            for i in 0..measures.len() {
                let in_measure: Vec<&Onset> =
                    events.iter().filter(|e| e.measure_index == i).collect();
                if in_measure.is_empty() {
                    t.push((0.0, 0.0, 0.0, 0.0));
                    continue;
                }
                let density = in_measure.len() as f64;
                let velocity = in_measure.iter().map(|e| e.velocity as f64).sum::<f64>()
                    / in_measure.len() as f64;
                let register = in_measure.iter().map(|e| e.pitch as f64).sum::<f64>()
                    / in_measure.len() as f64;
                let steps: Vec<i16> = in_measure
                    .windows(2)
                    .map(|p| (p[1].pitch as i16 - p[0].pitch as i16).abs() % 12)
                    .collect();
                let dissonance = if steps.is_empty() {
                    0.0
                } else {
                    steps.iter().filter(|&&s| s == 1 || s == 6 || s == 11).count() as f64
                        / steps.len() as f64
                };
                t.push((density, velocity, register, dissonance));
            }
            t
        };
        let norm = |get: fn(&(f64, f64, f64, f64)) -> f64, v: &(f64, f64, f64, f64)| {
            let lo = tuples.iter().map(get).fold(f64::INFINITY, f64::min);
            let hi = tuples.iter().map(get).fold(f64::NEG_INFINITY, f64::max);
            if hi - lo < 1e-9 {
                0.5
            } else {
                (get(v) - lo) / (hi - lo)
            }
        };
        tuples
            .iter()
            .map(|t| {
                0.35 * norm(|t| t.0, t)
                    + 0.25 * norm(|t| t.1, t)
                    + 0.2 * norm(|t| t.2, t)
                    + 0.2 * norm(|t| t.3, t)
            })
            .collect()
    };

    // Harmonic rhythm: how often the lowest pitch class (the implied root)
    // changes at measure boundaries.
    let roots: Vec<u8> = (0..measures.len())
        .filter_map(|i| {
            events
                .iter()
                .filter(|e| e.measure_index == i)
                .map(|e| e.pitch)
                .min()
                .map(|p| p % 12)
        })
        .collect();
    let harmonic_rhythm = if roots.len() < 2 {
        0.0
    } else {
        roots.windows(2).filter(|p| p[0] != p[1]).count() as f64 / (roots.len() - 1) as f64
    };

    CritiqueReport {
        groove_consistency,
        density_range,
        motif_repetition,
        top_motif,
        velocity_mean,
        velocity_std,
        syncopation,
        literal_repeats,
        varied_repeats,
        fresh_measures,
        tension,
        harmonic_rhythm,
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
    out.push_str(&format!(
        "  syncopation {:.0}%, development: {} literal / {} varied / {} fresh measures, \
         root changes {:.0}%\n",
        report.syncopation * 100.0,
        report.literal_repeats,
        report.varied_repeats,
        report.fresh_measures,
        report.harmonic_rhythm * 100.0,
    ));
    if report.tension.len() >= 4 {
        let peak = report
            .tension
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let mean = report.tension.iter().sum::<f64>() / report.tension.len() as f64;
        let std = (report
            .tension
            .iter()
            .map(|t| (t - mean).powi(2))
            .sum::<f64>()
            / report.tension.len() as f64)
            .sqrt();
        let curve: String = report
            .tension
            .iter()
            .map(|&t| {
                // 5-level sparkline of the tension curve
                ['.', ':', '-', '=', '#'][((t * 4.999) as usize).min(4)]
            })
            .collect();
        out.push_str(&format!(
            "  tension curve [{curve}] peak at relative measure {}\n",
            peak + 1
        ));
        if std < 0.08 {
            out.push_str("  ISSUE: flat tension curve — no build or release; vary density/dynamics/register across the range\n");
        }
    }
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
    if report.literal_repeats >= 2 && report.varied_repeats == 0 {
        out.push_str(
            "  ISSUE: copy-paste repetition — every repeat is literal; vary one \
             (tuxguitar_vary_riff: displace, invert, pedal, regroup...)\n",
        );
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
    fn gallop_rhythm_scores_consistent_not_erratic() {
        // Sixteenth-sixteenth-eighth gallop per beat: bimodal IOIs but a
        // perfectly repeating period-3 pattern.
        let mut steps = Vec::new();
        for beat in 0..4u64 {
            for (k, off) in [0u64, 240, 480].iter().enumerate() {
                let _ = k;
                steps.push((6u32, 0u32, 100u32, beat * 960 + off));
            }
        }
        let measures = vec![Measure {
            number: 1,
            start_tick: 960,
            key_signature: 0,
            beats: steps
                .iter()
                .map(|&(string, fret, velocity, off)| Beat {
                    start_tick: 960 + off,
                    voices: vec![Voice {
                        index: 0,
                        duration: Duration {
                            value: 16,
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
        }];
        let report = critique(&measures, STANDARD);
        assert!(
            report.groove_consistency > 0.85,
            "gallop must read as consistent, got {}",
            report.groove_consistency
        );
    }

    #[test]
    fn syncopation_scores_offbeat_material_higher() {
        // Straight 8ths: two on the quarters (0.0) + two offbeat 8ths (0.5).
        let eighths = vec![measure(1, &[(6, 0, 95), (6, 3, 95), (6, 5, 95), (5, 2, 95)])];
        assert_eq!(critique(&eighths, STANDARD).syncopation, 0.25);
        // Every onset pushed to a 16th position: maximally unmoored.
        let mut off = eighths.clone();
        for beat in &mut off[0].beats {
            beat.start_tick += 240;
        }
        assert_eq!(critique(&off, STANDARD).syncopation, 1.0);
    }

    #[test]
    fn motif_development_separates_literal_from_varied() {
        let base = [(6, 0, 95), (6, 3, 95), (6, 5, 95), (5, 2, 95)];
        let varied = [(6, 2, 95), (6, 5, 95), (6, 7, 95), (5, 4, 95)]; // same rhythm, new pitches
        let measures = vec![measure(1, &base), measure(2, &base), measure(3, &varied)];
        let report = critique(&measures, STANDARD);
        assert_eq!(report.literal_repeats, 1);
        assert_eq!(report.varied_repeats, 1);
        assert_eq!(report.fresh_measures, 1);
    }

    #[test]
    fn copy_paste_repetition_flags_as_issue() {
        let base = [(6, 0, 95), (6, 3, 95), (6, 5, 95), (5, 2, 95)];
        let measures = vec![measure(1, &base), measure(2, &base), measure(3, &base)];
        let report = critique(&measures, STANDARD);
        assert_eq!(report.literal_repeats, 2);
        assert!(describe(&report, "T1").contains("copy-paste"));
    }

    #[test]
    fn harmonic_rhythm_counts_root_changes() {
        let a = [(6, 0, 95), (6, 3, 95)]; // root E
        let b = [(6, 5, 95), (6, 8, 95)]; // root A
        let measures = vec![measure(1, &a), measure(2, &b), measure(3, &a)];
        let report = critique(&measures, STANDARD);
        assert!((report.harmonic_rhythm - 1.0).abs() < 1e-9); // changes at every boundary
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
