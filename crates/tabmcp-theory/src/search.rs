//! Constraint-guided riff generation: beam search over the rhythm-cell
//! alphabet and a scale's pitch space, scored while generating instead of
//! generate-then-evaluate. The search guarantees coherent complexity
//! (accent coverage, syncopation window, motif form); the AI client
//! steers taste through the constraints.

use tabmcp_model::{Beat, Duration, Measure, Note, NoteEffects, Tuplet, Voice};

use crate::cells::{RhythmCell, SpelledOnset, CELLS};
use crate::fingering::{optimize_monophonic, CostModel, Tuning};

pub struct RiffConstraints {
    /// Allowed MIDI pitches (scale intersected with the register).
    pub pitch_pool: Vec<u8>,
    /// Root pitch class 0..11 (accents gravitate here).
    pub root_pc: u8,
    /// Allowed cells; empty = the default metal alphabet.
    pub cells: Vec<&'static RhythmCell>,
    /// Length of each measure to fill, in ticks.
    pub measure_lens: Vec<u64>,
    /// Accent offsets within each measure (e.g. kick-unison points).
    pub accents: Vec<u64>,
    /// Target syncopation window 0..1.
    pub syncopation: (f64, f64),
    /// Target onsets per measure (lo, hi).
    pub notes_per_measure: (usize, usize),
    /// Longest allowed run of a repeated pitch.
    pub max_pitch_repeat: usize,
    /// Palm-mute unaccented low notes (chug feel).
    pub palm_mute_low: bool,
    /// Optional per-measure tension targets 0..1 (stretched over the
    /// range). High tension pulls density and register up; low tension
    /// thins the bar and sinks it. Empty = no coupling.
    pub tension: Vec<f64>,
    /// Phrase length in measures for cadences (0 = off). The last onset
    /// of every phrase resolves to the root pitch class.
    pub phrase_len: usize,
}

impl Default for RiffConstraints {
    fn default() -> Self {
        Self {
            pitch_pool: Vec::new(),
            root_pc: 9, // A
            cells: Vec::new(),
            measure_lens: vec![3840; 4],
            accents: vec![0],
            syncopation: (0.15, 0.55),
            notes_per_measure: (4, 10),
            max_pitch_repeat: 3,
            palm_mute_low: true,
            tension: Vec::new(),
            phrase_len: 4,
        }
    }
}

fn default_cells() -> Vec<&'static RhythmCell> {
    CELLS
        .iter()
        .filter(|c| {
            matches!(
                c.name,
                "8ths" | "16ths" | "gallop" | "reverse-gallop" | "tresillo" | "and-of-one"
                    | "sixteenth-rest-start" | "rest-8th" | "quarter"
            )
        })
        .collect()
}

fn onset_sync_weight(offset: u64) -> f64 {
    if offset % 960 == 0 {
        0.0
    } else if offset % 480 == 0 {
        0.5
    } else {
        1.0
    }
}

/// Beam-search a cell sequence filling one measure of `len` ticks.
fn solve_measure(
    len: u64,
    cells: &[&'static RhythmCell],
    accents: &[u64],
    syncopation: (f64, f64),
    notes_per_measure: (usize, usize),
    rotation: usize,
) -> Vec<SpelledOnset> {
    #[derive(Clone)]
    struct State {
        pos: u64,
        onsets: Vec<SpelledOnset>,
        cells_used: Vec<usize>,
    }
    let order: Vec<&'static RhythmCell> = {
        let mut v = cells.to_vec();
        let n = v.len();
        if n > 0 {
            v.rotate_left(rotation % n);
        }
        v
    };
    let score = |s: &State| -> f64 {
        let d = s.onsets.len();
        let (lo, hi) = notes_per_measure;
        let density_pen =
            (lo.saturating_sub(d) as f64 + d.saturating_sub(hi) as f64) * 1.0;
        let sync = if s.onsets.is_empty() {
            0.0
        } else {
            s.onsets.iter().map(|o| onset_sync_weight(o.offset)).sum::<f64>()
                / s.onsets.len() as f64
        };
        let sync_pen = if sync < syncopation.0 {
            (syncopation.0 - sync) * 10.0
        } else if sync > syncopation.1 {
            (sync - syncopation.1) * 10.0
        } else {
            0.0
        };
        let covered = accents
            .iter()
            .filter(|&&a| s.onsets.iter().any(|o| o.offset == a))
            .count() as f64;
        let breath_bonus = if s.pos < len || s.cells_used.iter().any(|&i| order[i].events.is_empty())
        {
            0.5
        } else {
            0.0
        };
        covered * 2.0 + breath_bonus - density_pen - sync_pen
    };

    let mut beam = vec![State {
        pos: 0,
        onsets: Vec::new(),
        cells_used: Vec::new(),
    }];
    let mut finished: Vec<(f64, State)> = Vec::new();
    while !beam.is_empty() {
        let mut next: Vec<(f64, State)> = Vec::new();
        for state in &beam {
            let mut expanded = false;
            for (ci, cell) in order.iter().enumerate() {
                if state.pos + cell.len <= len {
                    expanded = true;
                    let mut s = State {
                        pos: state.pos + cell.len,
                        onsets: state.onsets.clone(),
                        cells_used: state.cells_used.clone(),
                    };
                    for &(offset, value, dotted, enters, times) in cell.events {
                        s.onsets.push(SpelledOnset {
                            offset: state.pos + offset,
                            value,
                            dotted,
                            tuplet_enters: enters,
                            tuplet_times: times,
                        });
                    }
                    s.cells_used.push(ci);
                    let sc = score(&s);
                    next.push((sc, s));
                }
            }
            if !expanded {
                finished.push((score(state), state.clone()));
            }
        }
        next.sort_by(|a, b| b.0.total_cmp(&a.0));
        next.truncate(8);
        // States that reached the barline are terminal too.
        for (sc, s) in &next {
            if s.pos == len {
                finished.push((*sc, s.clone()));
            }
        }
        beam = next
            .into_iter()
            .filter(|(_, s)| s.pos < len)
            .map(|(_, s)| s)
            .collect();
    }
    finished.sort_by(|a, b| b.0.total_cmp(&a.0));
    finished
        .into_iter()
        .next()
        .map(|(_, s)| s.onsets)
        .unwrap_or_default()
}

struct FlatOnset {
    measure_index: usize,
    spelled: SpelledOnset,
    accented: bool,
    /// Per-measure tension 0..1 (0.5 when no target set).
    tension: f64,
    /// Phrase-final onset: the pitch search resolves these to the root.
    cadence: bool,
}

/// Assign pitches to onsets with a beam over the pool: roots on accents,
/// mostly stepwise motion with metal spice (b2/tritone bonus), bounded
/// repeat runs, and a motif bonus that echoes measure one's contour.
fn assign_pitches(onsets: &[FlatOnset], constraints: &RiffConstraints) -> Vec<u8> {
    let pool = &constraints.pitch_pool;
    if pool.is_empty() || onsets.is_empty() {
        return Vec::new();
    }
    let median = pool[pool.len() / 2] as f64;

    #[derive(Clone)]
    struct State {
        pitches: Vec<u8>,
        score: f64,
    }
    let mut beam = vec![State {
        pitches: Vec::new(),
        score: 0.0,
    }];
    // Reference contour: interval sequence of the first measure, filled as
    // the search discovers it (taken from the current best state).
    let first_measure_len = onsets
        .iter()
        .filter(|o| o.measure_index == 0)
        .count();
    for (i, onset) in onsets.iter().enumerate() {
        let mut next: Vec<State> = Vec::new();
        for state in &beam {
            for &pitch in pool {
                let mut score = state.score;
                if onset.accented {
                    if pitch % 12 == constraints.root_pc {
                        score += 2.0;
                    } else {
                        score -= 0.3;
                    }
                }
                // Cadence: phrase-final onsets resolve to the root.
                if onset.cadence {
                    if pitch % 12 == constraints.root_pc {
                        score += 3.0;
                    } else {
                        score -= 0.5;
                    }
                }
                // Tension pulls the register: high tension wants the top
                // of the pool, low tension the bottom.
                {
                    let lo = *pool.first().unwrap() as f64;
                    let hi = *pool.last().unwrap() as f64;
                    let anchor = lo + onset.tension * (hi - lo);
                    score -= (pitch as f64 - anchor).abs() * 0.035;
                }
                if let Some(&prev) = state.pitches.last() {
                    let iv = (pitch as i16 - prev as i16).abs();
                    score += match iv {
                        0 => 0.0,
                        1 | 2 => 0.8,
                        3 | 4 => 0.5,
                        5..=7 => 0.2,
                        _ => -0.8,
                    };
                    if iv % 12 == 6 || iv % 12 == 1 {
                        score += 0.3; // the spice
                    }
                    // Repeat-run control.
                    let run = state
                        .pitches
                        .iter()
                        .rev()
                        .take_while(|&&p| p == pitch)
                        .count();
                    if run >= constraints.max_pitch_repeat {
                        score -= 1.5 * (run - constraints.max_pitch_repeat + 1) as f64;
                    }
                    // Motif echo: same interval as the same position in
                    // measure one's contour.
                    if i >= first_measure_len && first_measure_len > 1 {
                        let echo_index = i % first_measure_len;
                        if echo_index > 0 && echo_index < state.pitches.len() {
                            let ref_iv = state.pitches[echo_index] as i16
                                - state.pitches[echo_index - 1] as i16;
                            if pitch as i16 - prev as i16 == ref_iv {
                                score += 0.6;
                            }
                        }
                    }
                }
                score -= (pitch as f64 - median).abs() * 0.02;
                let mut pitches = state.pitches.clone();
                pitches.push(pitch);
                next.push(State { pitches, score });
            }
        }
        next.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.pitches.last().cmp(&b.pitches.last()))
        });
        next.truncate(8);
        beam = next;
    }
    beam.into_iter().next().map(|s| s.pitches).unwrap_or_default()
}

/// Generate a riff under constraints. Returns wire measures (numbers start
/// at `first_number`, ticks at `first_start_tick`) plus an explanation.
pub fn generate_riff(
    constraints: &RiffConstraints,
    tuning: Tuning,
    max_fret: u32,
    first_number: u32,
    first_start_tick: u64,
) -> Result<(Vec<Measure>, String), String> {
    if constraints.pitch_pool.is_empty() {
        return Err("empty pitch pool - check scale and register".into());
    }
    let cells = if constraints.cells.is_empty() {
        default_cells()
    } else {
        constraints.cells.clone()
    };

    // Riff grammar: AABA' - slot A solved once, B gets a rotated alphabet,
    // the final A repeats A (pitch search then varies it via motif echo).
    let mut flat: Vec<FlatOnset> = Vec::new();
    let mut form: Vec<char> = Vec::new();
    let n = constraints.measure_lens.len();
    let tension_of = |mi: usize| -> f64 {
        if constraints.tension.is_empty() {
            0.5
        } else {
            constraints.tension[mi * constraints.tension.len() / n.max(1)]
                .clamp(0.0, 1.0)
        }
    };
    for (mi, &len) in constraints.measure_lens.iter().enumerate() {
        let slot = match mi % 4 {
            2 => 'B',
            _ => 'A',
        };
        form.push(slot);
        let rotation = if slot == 'B' { 3 } else { 0 };
        let tension = tension_of(mi);
        // Tension shapes density: high tension pushes the bar toward the
        // top of the density window, low tension thins it.
        let (dlo, dhi) = constraints.notes_per_measure;
        let density_window = if constraints.tension.is_empty() {
            (dlo, dhi)
        } else {
            let mid = dlo as f64 + tension * (dhi - dlo) as f64;
            (
                ((mid - 1.0).round() as usize).max(dlo.min(1)),
                ((mid + 1.5).round() as usize).min(dhi.max(2)),
            )
        };
        let onsets = solve_measure(
            len,
            &cells,
            &constraints.accents,
            constraints.syncopation,
            density_window,
            rotation,
        );
        let onset_count = onsets.len();
        let phrase_final_measure =
            constraints.phrase_len > 0 && (mi + 1) % constraints.phrase_len == 0;
        for (oi, spelled) in onsets.into_iter().enumerate() {
            let accented = constraints.accents.contains(&spelled.offset);
            flat.push(FlatOnset {
                measure_index: mi,
                spelled,
                accented,
                tension,
                cadence: phrase_final_measure && oi + 1 == onset_count,
            });
        }
    }
    if flat.is_empty() {
        return Err("no cell sequence satisfies the constraints in these measures".into());
    }

    let pitches = assign_pitches(&flat, constraints);
    let fingering = optimize_monophonic(&pitches, tuning, max_fret, &CostModel::metal())
        .map_err(|bad| format!("unplayable pitches at indices {bad:?} - widen the register"))?;

    // Assemble wire measures.
    let low_floor = constraints.pitch_pool[0].saturating_add(2);
    let mut measures: Vec<Measure> = Vec::new();
    let mut cursor = first_start_tick;
    for (mi, &len) in constraints.measure_lens.iter().enumerate() {
        measures.push(Measure {
            number: first_number + mi as u32,
            start_tick: cursor,
            key_signature: 0,
            beats: Vec::new(),
        });
        cursor += len;
    }
    let mut sync_sum = 0.0;
    for (i, onset) in flat.iter().enumerate() {
        let measure = &mut measures[onset.measure_index];
        let position = &fingering.path[i];
        let pitch = pitches[i];
        sync_sum += onset_sync_weight(onset.spelled.offset);
        let velocity = if onset.accented {
            112
        } else if onset.spelled.offset % 960 == 0 {
            100
        } else if onset.spelled.offset % 480 == 0 {
            94
        } else {
            88
        } + ((i * 7) % 5) as u32;
        let palm_mute =
            constraints.palm_mute_low && !onset.accented && pitch <= low_floor;
        measure.beats.push(Beat {
            start_tick: measure.start_tick + onset.spelled.offset,
            voices: vec![Voice {
                index: 0,
                duration: Duration {
                    value: onset.spelled.value,
                    dotted: onset.spelled.dotted,
                    double_dotted: false,
                    tuplet: Tuplet {
                        enters: onset.spelled.tuplet_enters,
                        times: onset.spelled.tuplet_times,
                    },
                },
                is_rest: false,
                notes: vec![Note {
                    string: position.string_number,
                    fret: position.fret,
                    velocity: velocity.min(127),
                    tied: false,
                    effects: NoteEffects {
                        palm_mute,
                        accent: onset.accented,
                        ..NoteEffects::default()
                    },
                }],
            }],
        });
    }
    for measure in &mut measures {
        measure.beats.sort_by_key(|b| b.start_tick);
    }

    let accents_hit = flat.iter().filter(|o| o.accented).count();
    let explanation = format!(
        "form {} | {} onsets, syncopation {:.0}%, {} accents on target offsets | \
         pool of {} pitches, root pc {} | fingering cost {:.1}",
        form.iter().collect::<String>(),
        flat.len(),
        sync_sum / flat.len() as f64 * 100.0,
        accents_hit,
        constraints.pitch_pool.len(),
        constraints.root_pc,
        fingering.cost,
    );
    Ok((measures, explanation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::critique;

    const SEVEN_STRING: &[(u32, u8)] = &[
        (1, 62),
        (2, 57),
        (3, 53),
        (4, 48),
        (5, 43),
        (6, 38),
        (7, 33),
    ];

    fn phrygian_dominant_pool(root: u8, lo: u8, hi: u8) -> Vec<u8> {
        // 1 b2 3 4 5 b6 b7
        let steps = [0u8, 1, 4, 5, 7, 8, 10];
        (lo..=hi)
            .filter(|p| steps.contains(&((p + 12 - root) % 12)))
            .collect()
    }

    #[test]
    fn generates_a_constrained_riff_that_scores_well() {
        let constraints = RiffConstraints {
            pitch_pool: phrygian_dominant_pool(9, 33, 50), // A1..D3
            root_pc: 9,
            measure_lens: vec![3840; 4],
            accents: vec![0, 1920],
            ..RiffConstraints::default()
        };
        let (measures, explanation) =
            generate_riff(&constraints, SEVEN_STRING, 24, 1, 960).expect("generates");
        assert_eq!(measures.len(), 4);
        let report = critique::critique(&measures, SEVEN_STRING);
        assert!(report.groove_consistency > 0.5, "{explanation}\n{report:?}");
        assert!(
            report.syncopation >= 0.10 && report.syncopation <= 0.60,
            "sync {} | {explanation}",
            report.syncopation
        );
        assert!(report.velocity_std >= 2.0, "velocities too flat");
        // Accents land where asked, on the root pitch class.
        let first = &measures[0].beats[0].voices[0].notes[0];
        assert!(first.effects.accent);
        // Deterministic: same constraints, same riff.
        let (again, _) = generate_riff(&constraints, SEVEN_STRING, 24, 1, 960).unwrap();
        assert_eq!(format!("{measures:?}"), format!("{again:?}"));
    }

    #[test]
    fn odd_meter_and_named_cells() {
        let constraints = RiffConstraints {
            pitch_pool: phrygian_dominant_pool(9, 33, 45),
            root_pc: 9,
            cells: vec![
                crate::cells::cell("tresillo").unwrap(),
                crate::cells::cell("8ths").unwrap(),
                crate::cells::cell("rest-8th").unwrap(),
            ],
            measure_lens: vec![3360; 2], // 7/8
            accents: vec![0],
            notes_per_measure: (3, 8),
            ..RiffConstraints::default()
        };
        let (measures, _) =
            generate_riff(&constraints, SEVEN_STRING, 24, 1, 960).expect("generates");
        for m in &measures {
            for b in &m.beats {
                assert!(b.start_tick - m.start_tick < 3360);
            }
            assert!(!m.beats.is_empty());
        }
    }

    #[test]
    fn tension_coupling_shapes_density_and_register() {
        let base = RiffConstraints {
            pitch_pool: phrygian_dominant_pool(9, 33, 52),
            root_pc: 9,
            measure_lens: vec![3840; 4],
            accents: vec![0],
            notes_per_measure: (3, 12),
            tension: vec![0.1, 0.4, 0.7, 1.0],
            phrase_len: 4,
            ..RiffConstraints::default()
        };
        let (measures, _) =
            generate_riff(&base, SEVEN_STRING, 24, 1, 960).expect("generates");
        let open: std::collections::HashMap<u32, u8> =
            SEVEN_STRING.iter().copied().collect();
        let stats: Vec<(usize, f64)> = measures
            .iter()
            .map(|m| {
                let pitches: Vec<f64> = m
                    .beats
                    .iter()
                    .flat_map(|b| &b.voices)
                    .flat_map(|v| &v.notes)
                    .map(|n| (open[&n.string] + n.fret as u8) as f64)
                    .collect();
                let mean = pitches.iter().sum::<f64>() / pitches.len().max(1) as f64;
                (pitches.len(), mean)
            })
            .collect();
        // Rising tension: the last bar is denser and higher than the first.
        assert!(stats[3].0 > stats[0].0, "{stats:?}");
        assert!(stats[3].1 > stats[0].1, "{stats:?}");
        // Cadence: the final onset lands on the root pitch class.
        let last = measures[3].beats.last().unwrap().voices[0].notes[0].clone();
        assert_eq!((open[&last.string] + last.fret as u8) % 12, 9);
    }

    #[test]
    fn empty_pool_is_an_error() {
        let constraints = RiffConstraints {
            pitch_pool: vec![],
            ..RiffConstraints::default()
        };
        assert!(generate_riff(&constraints, SEVEN_STRING, 24, 1, 960).is_err());
    }
}
