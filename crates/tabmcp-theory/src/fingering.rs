//! Monophonic fingering optimization.
//!
//! Pipeline: pitch sequence → all valid string/fret candidates per pitch →
//! dynamic programming over transitions → lowest-cost playable path.
//!
//! The cost model implements the MVP function from the project plan: fret
//! movement, string movement, position-shift penalty for large jumps,
//! string-skipping penalty, mild low-position and open-string preference.
//! Chord fingering and technique-aware costs (sweeps, legato, ...) come later.

/// A concrete place to play a pitch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// String number, 1-based, 1 = highest-sounding string.
    pub string_number: u32,
    pub fret: u32,
}

/// Tuning as (string_number, open_pitch), string 1 first.
pub type Tuning<'a> = &'a [(u32, u8)];

#[derive(Debug, Clone)]
pub struct FingeringResult {
    pub path: Vec<Position>,
    /// Total transition+node cost of the chosen path (lower = easier).
    pub cost: f64,
}

/// Tunable cost model — the pluggable part of the pipeline. Swap the
/// weights (or construct genre presets) without touching candidate
/// generation or the search itself. All weights are per-unit costs.
#[derive(Debug, Clone)]
pub struct CostModel {
    /// Cost per fret of movement between consecutive fretted notes.
    pub fret_move: f64,
    /// Cost per string crossed between consecutive notes.
    pub string_move: f64,
    /// Fret distance beyond which a move counts as a position shift.
    pub stretch_span: u32,
    /// Flat penalty for shifting the whole hand position.
    pub position_shift: f64,
    /// Extra penalty when more than two strings are crossed at once.
    pub string_skip: f64,
    /// Cost per string crossed when one of the notes is an open string.
    pub open_transition: f64,
    /// Per-fret cost nudging lines toward lower positions.
    pub low_position_bias: f64,
    /// Restrict fretted notes to this fret window (open strings stay
    /// allowed). E.g. Some((5, 12)) for a user who wants to stay mid-neck.
    pub fret_range: Option<(u32, u32)>,
}

impl Default for CostModel {
    fn default() -> Self {
        CostModel {
            fret_move: 0.6,
            string_move: 0.35,
            stretch_span: 4,
            position_shift: 2.5,
            string_skip: 0.8,
            open_transition: 0.25,
            low_position_bias: 0.05,
            fret_range: None,
        }
    }
}

impl CostModel {
    /// All places a pitch can be played on the given tuning under this model.
    fn candidates(&self, pitch: u8, tuning: Tuning, max_fret: u32) -> Vec<Position> {
        tuning
            .iter()
            .filter_map(|&(string_number, open)| {
                let fret = pitch as i32 - open as i32;
                if fret < 0 || fret > max_fret as i32 {
                    return None;
                }
                if let Some((lo, hi)) = self.fret_range {
                    if fret != 0 && (fret < lo as i32 || fret > hi as i32) {
                        return None;
                    }
                }
                Some(Position {
                    string_number,
                    fret: fret as u32,
                })
            })
            .collect()
    }

    fn node_cost(&self, p: Position) -> f64 {
        if p.fret == 0 {
            0.0 // open strings are free
        } else {
            self.low_position_bias * p.fret as f64
        }
    }

    fn transition_cost(&self, a: Position, b: Position) -> f64 {
        let string_move = (a.string_number as i64 - b.string_number as i64).unsigned_abs() as f64;
        if a.fret == 0 || b.fret == 0 {
            // Moves through an open string barely constrain the fretting hand.
            return self.open_transition * string_move;
        }
        let fret_move = (a.fret as i64 - b.fret as i64).unsigned_abs() as f64;
        let mut cost = self.fret_move * fret_move + self.string_move * string_move;
        if fret_move > self.stretch_span as f64 {
            cost += self.position_shift; // leaving the hand position entirely
        }
        if string_move > 2.0 {
            cost += self.string_skip; // string skipping
        }
        cost
    }
}

/// Where a path's effort comes from — the raw material for explanations.
#[derive(Debug, Clone, Default)]
pub struct EffortBreakdown {
    pub total_cost: f64,
    /// Sum of fret distances between consecutive fretted notes.
    pub fret_movement: u64,
    /// Sum of strings crossed between consecutive notes.
    pub string_movement: u64,
    /// Number of whole-hand position shifts (jumps beyond the stretch span).
    pub position_shifts: u32,
    /// Number of string-skip moves (crossing more than 2 strings).
    pub string_skips: u32,
    /// Number of open-string notes used.
    pub open_strings: u32,
    /// Highest fret touched.
    pub max_fret: u32,
}

/// Analyze a path under a cost model.
pub fn breakdown(path: &[Position], model: &CostModel) -> EffortBreakdown {
    let mut b = EffortBreakdown {
        total_cost: path_cost_with(path, model),
        open_strings: path.iter().filter(|p| p.fret == 0).count() as u32,
        max_fret: path.iter().map(|p| p.fret).max().unwrap_or(0),
        ..Default::default()
    };
    for pair in path.windows(2) {
        let string_move =
            (pair[0].string_number as i64 - pair[1].string_number as i64).unsigned_abs();
        b.string_movement += string_move;
        if pair[0].fret > 0 && pair[1].fret > 0 {
            let fret_move = (pair[0].fret as i64 - pair[1].fret as i64).unsigned_abs();
            b.fret_movement += fret_move;
            if fret_move > model.stretch_span as u64 {
                b.position_shifts += 1;
            }
            if string_move > 2 {
                b.string_skips += 1;
            }
        }
    }
    b
}

/// Human-readable reasons why `new` beats `old` — the "why did you choose
/// this fingering?" answer surfaced to AI clients.
pub fn explain_improvement(old: &EffortBreakdown, new: &EffortBreakdown) -> Vec<String> {
    let mut reasons = Vec::new();
    if old.total_cost > 0.0 && new.total_cost < old.total_cost {
        let percent = ((old.total_cost - new.total_cost) / old.total_cost * 100.0).round();
        reasons.push(format!(
            "{percent:.0}% less hand effort overall ({:.1} -> {:.1})",
            old.total_cost, new.total_cost
        ));
    }
    if new.fret_movement < old.fret_movement {
        reasons.push(format!(
            "fret-hand travel reduced from {} to {} frets",
            old.fret_movement, new.fret_movement
        ));
    }
    if new.position_shifts < old.position_shifts {
        reasons.push(format!(
            "{} whole-hand position shift(s) eliminated ({} -> {})",
            old.position_shifts - new.position_shifts,
            old.position_shifts,
            new.position_shifts
        ));
    }
    if new.string_skips < old.string_skips {
        reasons.push(format!(
            "avoids {} string-skip move(s)",
            old.string_skips - new.string_skips
        ));
    }
    if new.open_strings > old.open_strings {
        reasons.push(format!(
            "uses {} open string(s) instead of {}",
            new.open_strings, old.open_strings
        ));
    }
    if new.max_fret < old.max_fret {
        reasons.push(format!(
            "stays below fret {} (was reaching fret {})",
            new.max_fret + 1,
            old.max_fret
        ));
    }
    if reasons.is_empty() {
        reasons.push("the current fingering is already near-optimal".to_string());
    }
    reasons
}

/// Total cost of an existing path under a model (for before/after compares).
pub fn path_cost_with(path: &[Position], model: &CostModel) -> f64 {
    let mut total: f64 = path.iter().map(|&p| model.node_cost(p)).sum();
    total += path
        .windows(2)
        .map(|w| model.transition_cost(w[0], w[1]))
        .sum::<f64>();
    total
}

/// Total cost under the default model.
pub fn path_cost(path: &[Position]) -> f64 {
    path_cost_with(path, &CostModel::default())
}

/// One moment in a passage: a single pitch or a chord of pitches.
#[derive(Debug, Clone)]
pub enum Step {
    Mono(u8),
    /// Pitches sorted ascending; result positions align to this order.
    Chord(Vec<u8>),
}

/// Result of optimizing a mixed mono/chord passage: one position set per
/// step, aligned to the step's pitch order.
#[derive(Debug, Clone)]
pub struct StepsResult {
    pub path: Vec<Vec<Position>>,
    pub cost: f64,
}

/// Fretted-hand "center" of a position set (open strings excluded).
fn centroid(set: &[Position]) -> (f64, f64) {
    let fretted: Vec<&Position> = set.iter().filter(|p| p.fret > 0).collect();
    if fretted.is_empty() {
        let avg_string =
            set.iter().map(|p| p.string_number as f64).sum::<f64>() / set.len().max(1) as f64;
        return (0.0, avg_string);
    }
    (
        fretted.iter().map(|p| p.fret as f64).sum::<f64>() / fretted.len() as f64,
        fretted.iter().map(|p| p.string_number as f64).sum::<f64>() / fretted.len() as f64,
    )
}

fn set_transition_cost(model: &CostModel, a: &[Position], b: &[Position]) -> f64 {
    let (fret_a, string_a) = centroid(a);
    let (fret_b, string_b) = centroid(b);
    if fret_a == 0.0 || fret_b == 0.0 {
        return model.open_transition * (string_a - string_b).abs();
    }
    let fret_move = (fret_a - fret_b).abs();
    let mut cost = model.fret_move * fret_move + model.string_move * (string_a - string_b).abs();
    if fret_move > model.stretch_span as f64 {
        cost += model.position_shift;
    }
    cost
}

fn set_node_cost(model: &CostModel, set: &[Position]) -> f64 {
    let base: f64 = set.iter().map(|&p| model.node_cost(p)).sum();
    let fretted: Vec<u32> = set.iter().filter(|p| p.fret > 0).map(|p| p.fret).collect();
    let span = match (fretted.iter().max(), fretted.iter().min()) {
        (Some(max), Some(min)) => (max - min) as f64,
        _ => 0.0,
    };
    // Wide chord voicings hurt: charge for stretch beyond 3 frets.
    base + if span > 3.0 { (span - 3.0) * 1.5 } else { 0.0 }
}

/// All playable voicings for a chord (pitches ascending), positions aligned
/// to the pitch order, capped to the `limit` cheapest by node cost.
fn chord_voicings(
    pitches: &[u8],
    tuning: Tuning,
    max_fret: u32,
    model: &CostModel,
    limit: usize,
) -> Vec<Vec<Position>> {
    #[allow(clippy::too_many_arguments)] // recursion carries its whole state
    fn assign(
        pitches: &[u8],
        index: usize,
        tuning: Tuning,
        max_fret: u32,
        model: &CostModel,
        used: &mut Vec<u32>,
        current: &mut Vec<Position>,
        out: &mut Vec<Vec<Position>>,
    ) {
        if index == pitches.len() {
            out.push(current.clone());
            return;
        }
        for candidate in model.candidates(pitches[index], tuning, max_fret) {
            if used.contains(&candidate.string_number) {
                continue;
            }
            // Prune impossible stretches early.
            let fretted: Vec<u32> = current
                .iter()
                .chain(std::iter::once(&candidate))
                .filter(|p| p.fret > 0)
                .map(|p| p.fret)
                .collect();
            if let (Some(&max), Some(&min)) = (fretted.iter().max(), fretted.iter().min()) {
                if max - min > 5 {
                    continue;
                }
            }
            used.push(candidate.string_number);
            current.push(candidate);
            assign(
                pitches,
                index + 1,
                tuning,
                max_fret,
                model,
                used,
                current,
                out,
            );
            current.pop();
            used.pop();
        }
    }
    let mut out = Vec::new();
    assign(
        pitches,
        0,
        tuning,
        max_fret,
        model,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut out,
    );
    out.sort_by(|a, b| set_node_cost(model, a).total_cmp(&set_node_cost(model, b)));
    out.truncate(limit);
    out
}

/// Optimize a mixed passage of single notes and chords: same candidate ->
/// cost -> DP pipeline, with chord voicings as multi-position candidates.
pub fn optimize_steps(
    steps: &[Step],
    tuning: Tuning,
    max_fret: u32,
    model: &CostModel,
) -> Result<StepsResult, Vec<usize>> {
    if steps.is_empty() {
        return Ok(StepsResult {
            path: Vec::new(),
            cost: 0.0,
        });
    }
    let mut candidates: Vec<Vec<Vec<Position>>> = Vec::with_capacity(steps.len());
    let mut unplayable = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let sets: Vec<Vec<Position>> = match step {
            Step::Mono(pitch) => model
                .candidates(*pitch, tuning, max_fret)
                .into_iter()
                .map(|p| vec![p])
                .collect(),
            Step::Chord(pitches) => chord_voicings(pitches, tuning, max_fret, model, 6),
        };
        if sets.is_empty() {
            unplayable.push(i);
        }
        candidates.push(sets);
    }
    if !unplayable.is_empty() {
        return Err(unplayable);
    }

    let mut costs: Vec<f64> = candidates[0]
        .iter()
        .map(|s| set_node_cost(model, s))
        .collect();
    let mut parents: Vec<Vec<usize>> = Vec::with_capacity(steps.len());
    for step in 1..steps.len() {
        let mut next = vec![f64::INFINITY; candidates[step].len()];
        let mut back = vec![0usize; candidates[step].len()];
        for (ci, cur) in candidates[step].iter().enumerate() {
            for (pi, prev) in candidates[step - 1].iter().enumerate() {
                let cost =
                    costs[pi] + set_transition_cost(model, prev, cur) + set_node_cost(model, cur);
                if cost < next[ci] {
                    next[ci] = cost;
                    back[ci] = pi;
                }
            }
        }
        costs = next;
        parents.push(back);
    }
    let (mut best, best_cost) = costs
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, &c)| (i, c))
        .expect("non-empty");
    let mut path = vec![candidates[steps.len() - 1][best].clone()];
    for step in (1..steps.len()).rev() {
        best = parents[step - 1][best];
        path.push(candidates[step - 1][best].clone());
    }
    path.reverse();
    Ok(StepsResult {
        path,
        cost: best_cost,
    })
}

/// Find the lowest-cost way to play a monophonic pitch sequence.
///
/// Returns `Err(indices)` (positions in the input) for pitches that cannot
/// be played anywhere on the tuning at all.
pub fn optimize_monophonic(
    pitches: &[u8],
    tuning: Tuning,
    max_fret: u32,
    model: &CostModel,
) -> Result<FingeringResult, Vec<usize>> {
    if pitches.is_empty() {
        return Ok(FingeringResult {
            path: Vec::new(),
            cost: 0.0,
        });
    }
    let all_candidates: Vec<Vec<Position>> = pitches
        .iter()
        .map(|&p| model.candidates(p, tuning, max_fret))
        .collect();
    let unplayable: Vec<usize> = all_candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_empty())
        .map(|(i, _)| i)
        .collect();
    if !unplayable.is_empty() {
        return Err(unplayable);
    }

    // dp[c] = (cost of best path ending at candidate c, backpointer per step)
    let mut costs: Vec<f64> = all_candidates[0]
        .iter()
        .map(|&p| model.node_cost(p))
        .collect();
    let mut parents: Vec<Vec<usize>> = Vec::with_capacity(pitches.len());

    for step in 1..pitches.len() {
        let mut next_costs = vec![f64::INFINITY; all_candidates[step].len()];
        let mut step_parents = vec![0usize; all_candidates[step].len()];
        for (ci, &cur) in all_candidates[step].iter().enumerate() {
            for (pi, &prev) in all_candidates[step - 1].iter().enumerate() {
                let cost = costs[pi] + model.transition_cost(prev, cur) + model.node_cost(cur);
                if cost < next_costs[ci] {
                    next_costs[ci] = cost;
                    step_parents[ci] = pi;
                }
            }
        }
        costs = next_costs;
        parents.push(step_parents);
    }

    // Reconstruct from the cheapest final candidate.
    let (mut best_index, best_cost) = costs
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, &c)| (i, c))
        .expect("non-empty candidates");
    let mut path = vec![all_candidates[pitches.len() - 1][best_index]];
    for step in (1..pitches.len()).rev() {
        best_index = parents[step - 1][best_index];
        path.push(all_candidates[step - 1][best_index]);
    }
    path.reverse();

    Ok(FingeringResult {
        path,
        cost: best_cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard tuning, string 1 (E4) first.
    const STANDARD: &[(u32, u8)] = &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];

    #[test]
    fn scale_run_stays_in_one_position() {
        // C major scale, C4..C5.
        let pitches = [60u8, 62, 64, 65, 67, 69, 71, 72];
        let result =
            optimize_monophonic(&pitches, STANDARD, 24, &CostModel::default()).expect("playable");
        assert_eq!(result.path.len(), pitches.len());
        // No adjacent same-string jump should leave the hand position.
        for pair in result.path.windows(2) {
            if pair[0].string_number == pair[1].string_number
                && pair[0].fret > 0
                && pair[1].fret > 0
            {
                let jump = (pair[0].fret as i64 - pair[1].fret as i64).unsigned_abs();
                assert!(jump <= 4, "position jump within a string: {:?}", pair);
            }
        }
        // The whole run should sit in a compact window (not sprawl to fret 17).
        let max = result.path.iter().map(|p| p.fret).max().unwrap();
        assert!(max <= 10, "expected a compact position, got max fret {max}");
    }

    #[test]
    fn open_strings_are_used_when_free() {
        // The open strings themselves.
        let pitches = [40u8, 45, 50, 55, 59, 64];
        let result =
            optimize_monophonic(&pitches, STANDARD, 24, &CostModel::default()).expect("playable");
        assert!(
            result.path.iter().all(|p| p.fret == 0),
            "expected all open strings, got {:?}",
            result.path
        );
    }

    #[test]
    fn repeated_pitch_keeps_its_position() {
        let pitches = [57u8, 57, 57, 57];
        let result =
            optimize_monophonic(&pitches, STANDARD, 24, &CostModel::default()).expect("playable");
        let first = result.path[0];
        assert!(result.path.iter().all(|&p| p == first));
    }

    #[test]
    fn unplayable_pitches_are_reported_by_index() {
        let pitches = [60u8, 20, 62, 10];
        match optimize_monophonic(&pitches, STANDARD, 24, &CostModel::default()) {
            Err(indices) => assert_eq!(indices, vec![1, 3]),
            Ok(_) => panic!("pitches below the tuning must be unplayable"),
        }
    }

    #[test]
    fn optimized_path_is_never_worse_than_naive_lowest_string() {
        // A melody that tempts a naive mapper onto one string with big jumps.
        let pitches = [45u8, 52, 48, 55, 50, 57, 52, 59];
        let result =
            optimize_monophonic(&pitches, STANDARD, 24, &CostModel::default()).expect("playable");
        // Naive: always the lowest string that can play it.
        let naive: Vec<Position> = pitches
            .iter()
            .map(|&p| {
                CostModel::default()
                    .candidates(p, STANDARD, 24)
                    .into_iter()
                    .max_by_key(|c| c.string_number)
                    .unwrap()
            })
            .collect();
        assert!(
            result.cost <= path_cost(&naive) + 1e-9,
            "DP path ({}) must beat naive ({})",
            result.cost,
            path_cost(&naive)
        );
    }
}

#[cfg(test)]
mod chord_tests {
    use super::*;

    const STANDARD: &[(u32, u8)] = &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];

    #[test]
    fn chord_passage_gets_playable_compact_voicings() {
        // E5 power chord -> A5 -> single G3 -> D5 chord.
        let steps = [
            Step::Chord(vec![40, 47]),
            Step::Chord(vec![45, 52]),
            Step::Mono(55),
            Step::Chord(vec![50, 57]),
        ];
        let result = optimize_steps(&steps, STANDARD, 24, &CostModel::default()).expect("playable");
        assert_eq!(result.path.len(), 4);
        for (step, set) in steps.iter().zip(&result.path) {
            // Distinct strings, correct pitch count, tight span.
            let mut strings: Vec<u32> = set.iter().map(|p| p.string_number).collect();
            strings.dedup();
            match step {
                Step::Chord(p) => assert_eq!(set.len(), p.len()),
                Step::Mono(_) => assert_eq!(set.len(), 1),
            }
            assert_eq!(strings.len(), set.len(), "strings must be unique: {set:?}");
            let fretted: Vec<u32> = set.iter().filter(|p| p.fret > 0).map(|p| p.fret).collect();
            if let (Some(max), Some(min)) = (fretted.iter().max(), fretted.iter().min()) {
                assert!(max - min <= 5, "span too wide: {set:?}");
            }
        }
    }

    #[test]
    fn unvoiceable_chord_reports_step_index() {
        // Three pitches below the lowest string cannot be voiced.
        let steps = [Step::Mono(45), Step::Chord(vec![20, 21, 22])];
        match optimize_steps(&steps, STANDARD, 24, &CostModel::default()) {
            Err(indices) => assert_eq!(indices, vec![1]),
            Ok(_) => panic!("expected unplayable"),
        }
    }
}
