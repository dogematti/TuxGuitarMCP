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
