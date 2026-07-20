//! Lick-cell lead generator: real lead lines from pitch-shape cells
//! (the melodic analogue of the rhythm-cell catalog). A contour plan
//! spans the phrase arc, question/answer periods pair phrases, the
//! fingering optimizer keeps it playable, and a difficulty cap keeps it
//! honest. This is solos, intros, and hooks - not gap-filling.

use tabmcp_model::{Beat, Duration, Measure, Note, NoteEffects, Tuplet, Voice};

use crate::difficulty;
use crate::fingering::{optimize_monophonic, CostModel, Tuning};

/// A pitch-shape cell: scale-degree steps relative to the entry note,
/// with a rhythm (ticks per note). Shapes are transposed to wherever the
/// contour wants them.
pub struct LickCell {
    pub name: &'static str,
    /// Scale-index deltas from the previous note (0 = repeat).
    pub steps: &'static [i8],
    /// Tick length per note (uniform inside a cell).
    pub note_ticks: u64,
    /// What it feels like.
    pub feel: &'static str,
}

pub const LICK_CELLS: &[LickCell] = &[
    LickCell { name: "run-up", steps: &[1, 1, 1, 1], note_ticks: 240, feel: "scalar sprint upward" },
    LickCell { name: "run-down", steps: &[-1, -1, -1, -1], note_ticks: 240, feel: "scalar sprint downward" },
    LickCell { name: "enclosure", steps: &[1, -2, 1], note_ticks: 240, feel: "circle the target before landing" },
    LickCell { name: "pedal-return", steps: &[2, -2, 3, -3], note_ticks: 240, feel: "bounce off a pedal tone" },
    LickCell { name: "leap-fall", steps: &[4, -1, -1, -1], note_ticks: 240, feel: "jump high then walk down" },
    LickCell { name: "waves", steps: &[2, -1, 2, -1], note_ticks: 240, feel: "rising zigzag" },
    LickCell { name: "hold", steps: &[0], note_ticks: 960, feel: "land and sing (gets vibrato)" },
    LickCell { name: "call", steps: &[1, 1, -1], note_ticks: 480, feel: "questioning 8ths, ends unresolved" },
    LickCell { name: "answer", steps: &[-1, -1, 1], note_ticks: 480, feel: "answering 8ths, ends settled" },
];

pub struct LeadPlan {
    /// Contour targets per measure, 0..1 of the register span.
    pub contour: Vec<f64>,
    /// Max difficulty 1-10 the line may reach.
    pub max_difficulty: f64,
}

impl Default for LeadPlan {
    fn default() -> Self {
        Self {
            // Classic solo arc: establish low, build, peak at ~3/4, resolve.
            contour: vec![0.2, 0.35, 0.5, 0.45, 0.6, 0.8, 1.0, 0.4],
            max_difficulty: 7.0,
        }
    }
}

fn scale_pool(root_pc: u8, steps: &[u8], lo: u8, hi: u8) -> Vec<u8> {
    (lo..=hi)
        .filter(|p| steps.contains(&((p + 12 - root_pc) % 12)))
        .collect()
}

/// Generate a lead line over `measure_lens` (ticks each), in the scale,
/// within [register_lo, register_hi]. Question/answer periods: odd
/// measures get "call"-flavored ends, even measures "answer"-flavored,
/// and the final measure lands on the root with a hold.
pub fn generate_lead(
    root_pc: u8,
    scale_steps: &[u8],
    register_lo: u8,
    register_hi: u8,
    measure_lens: &[u64],
    plan: &LeadPlan,
    tuning: Tuning,
    max_fret: u32,
    tempo_bpm: u32,
    first_number: u32,
    first_start_tick: u64,
) -> Result<(Vec<Measure>, String), String> {
    let pool = scale_pool(root_pc, scale_steps, register_lo, register_hi);
    if pool.len() < 6 {
        return Err("register too narrow for a lead - widen it".into());
    }
    let span = pool.len() - 1;

    // Pick cells per measure: contour rising -> upward cells, falling ->
    // downward, flat -> waves/pedal; measure end gets call/answer; last
    // measure resolves with hold on the root.
    let mut all_notes: Vec<(usize, u64, u64, u8)> = Vec::new(); // (measure, offset, ticks, pitch)
    let mut pool_index: usize = (plan.contour.first().copied().unwrap_or(0.3)
        * span as f64) as usize;
    let n = measure_lens.len();
    for (mi, &len) in measure_lens.iter().enumerate() {
        let target = plan
            .contour
            .get(mi * plan.contour.len() / n.max(1))
            .copied()
            .unwrap_or(0.5);
        let target_index = (target * span as f64) as usize;
        let last_measure = mi + 1 == n;
        let mut offset = 0u64;
        // Breathing: phrases start after a beat of rest on even measures.
        if mi % 2 == 1 {
            offset = 480;
        }
        while offset < len {
            let remaining = len - offset;
            let rising = target_index > pool_index;
            let cell = if last_measure && remaining <= 1920 {
                &LICK_CELLS[6] // hold - the landing
            } else if remaining <= 1440 {
                // Phrase end: question on odd measures, answer on even.
                if mi % 2 == 0 { &LICK_CELLS[7] } else { &LICK_CELLS[8] }
            } else if pool_index.abs_diff(target_index) <= 1 {
                if (mi + offset as usize / 960) % 2 == 0 {
                    &LICK_CELLS[5] // waves
                } else {
                    &LICK_CELLS[3] // pedal-return
                }
            } else if rising {
                if remaining >= 1920 && (offset / 960) % 2 == 1 {
                    &LICK_CELLS[4] // leap-fall spice
                } else {
                    &LICK_CELLS[0] // run-up
                }
            } else {
                if (offset / 960) % 3 == 2 {
                    &LICK_CELLS[2] // enclosure
                } else {
                    &LICK_CELLS[1] // run-down
                }
            };
            for &step in cell.steps {
                if offset >= len {
                    break;
                }
                let next = pool_index as i64 + step as i64;
                pool_index = next.clamp(0, span as i64) as usize;
                let mut pitch = pool[pool_index];
                // The very last note lands on the root pitch class.
                if last_measure && offset + cell.note_ticks >= len {
                    if let Some(&root_pitch) = pool
                        .iter()
                        .filter(|p| **p % 12 == root_pc)
                        .min_by_key(|p| (**p as i16 - pitch as i16).abs())
                    {
                        pitch = root_pitch;
                        pool_index = pool.iter().position(|&p| p == pitch).unwrap_or(pool_index);
                    }
                }
                all_notes.push((mi, offset, cell.note_ticks, pitch));
                offset += cell.note_ticks;
            }
        }
    }
    if all_notes.is_empty() {
        return Err("no notes generated - check the measure range".into());
    }

    // Fingering + assembly.
    let pitches: Vec<u8> = all_notes.iter().map(|&(_, _, _, p)| p).collect();
    let fingering = optimize_monophonic(&pitches, tuning, max_fret, &CostModel::default())
        .map_err(|bad| format!("lead unplayable at indices {bad:?} - widen the register"))?;
    let mut measures: Vec<Measure> = Vec::with_capacity(n);
    let mut cursor = first_start_tick;
    for (mi, &len) in measure_lens.iter().enumerate() {
        measures.push(Measure {
            number: first_number + mi as u32,
            start_tick: cursor,
            key_signature: 0,
            beats: Vec::new(),
        });
        cursor += len;
    }
    for (i, &(mi, offset, ticks, _)) in all_notes.iter().enumerate() {
        let position = &fingering.path[i];
        let value = (3840 / ticks.max(120)) as u32;
        let velocity = 88
            + ((plan.contour[mi * plan.contour.len() / n.max(1)] * 24.0) as u32)
            + ((i * 3) % 5) as u32;
        let measure_start = measures[mi].start_tick;
        measures[mi].beats.push(Beat {
            start_tick: measure_start + offset,
            voices: vec![Voice {
                index: 0,
                duration: Duration {
                    value: value.clamp(1, 64),
                    dotted: false,
                    double_dotted: false,
                    tuplet: Tuplet { enters: 1, times: 1 },
                },
                is_rest: false,
                notes: vec![Note {
                    string: position.string_number,
                    fret: position.fret,
                    velocity: velocity.min(127),
                    tied: false,
                    effects: NoteEffects::default(),
                }],
            }],
        });
    }

    // Difficulty cap: if too hard, thin 16th runs to 8ths and re-check.
    let mut report = difficulty::analyze(&measures, tuning, tempo_bpm);
    let mut thinned = false;
    if report.score > plan.max_difficulty {
        for measure in &mut measures {
            let mut keep = true;
            measure.beats.retain(|beat| {
                let is_sixteenth =
                    beat.voices.first().map(|v| v.duration.value >= 16).unwrap_or(false);
                if is_sixteenth {
                    keep = !keep;
                    keep // drop every other 16th
                } else {
                    true
                }
            });
        }
        thinned = true;
        report = difficulty::analyze(&measures, tuning, tempo_bpm);
    }

    let description = format!(
        "lead over {} bars: contour arc peaking at {:.0}%, question/answer phrase \
         ends, resolves to the root; difficulty {:.1}/10{}",
        n,
        plan.contour.iter().cloned().fold(0.0f64, f64::max) * 100.0,
        report.score,
        if thinned { " (thinned 16ths to meet the difficulty cap)" } else { "" },
    );
    Ok((measures, description))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEVEN: &[(u32, u8)] = &[
        (1, 62),
        (2, 57),
        (3, 53),
        (4, 48),
        (5, 43),
        (6, 38),
        (7, 33),
    ];
    const MINOR: &[u8] = &[0, 2, 3, 5, 7, 8, 10];

    #[test]
    fn lead_follows_contour_and_resolves_to_root() {
        let (measures, description) = generate_lead(
            9, // A
            MINOR,
            45,
            62,
            &[3840; 8],
            &LeadPlan::default(),
            SEVEN,
            24,
            140,
            1,
            960,
        )
        .expect("generates");
        assert_eq!(measures.len(), 8);
        // Last sounding note is the root pitch class.
        let open: std::collections::HashMap<u32, u8> = SEVEN.iter().copied().collect();
        let last = measures
            .last()
            .unwrap()
            .beats
            .last()
            .unwrap()
            .voices[0]
            .notes[0]
            .clone();
        let pitch = open[&last.string] + last.fret as u8;
        assert_eq!(pitch % 12, 9, "{description}");
        // The contour peak lands in the back half: highest pitch after bar 4.
        let mut peak = (0usize, 0u8);
        for (mi, measure) in measures.iter().enumerate() {
            for beat in &measure.beats {
                for note in &beat.voices[0].notes {
                    let p = open[&note.string] + note.fret as u8;
                    if p > peak.1 {
                        peak = (mi, p);
                    }
                }
            }
        }
        assert!(peak.0 >= 4, "peak in bar {} - {description}", peak.0 + 1);
        // Even measures breathe (start with a rest).
        assert!(measures[1]
            .beats
            .first()
            .map(|b| b.start_tick - measures[1].start_tick >= 480)
            .unwrap_or(true));
    }

    #[test]
    fn difficulty_cap_thins_the_line() {
        let easy_plan = LeadPlan {
            contour: vec![0.3, 1.0, 1.0, 0.2],
            max_difficulty: 2.0, // absurd cap forces thinning
        };
        let (_, description) = generate_lead(
            9, MINOR, 45, 62, &[3840; 4], &easy_plan, SEVEN, 24, 200, 1, 960,
        )
        .expect("generates");
        assert!(description.contains("thinned"), "{description}");
    }

    #[test]
    fn narrow_register_errors() {
        assert!(generate_lead(9, MINOR, 45, 48, &[3840; 2], &LeadPlan::default(), SEVEN, 24, 120, 1, 960).is_err());
    }
}
