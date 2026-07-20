//! Difficulty analyzer: how hard is this passage for a human player, 1-10,
//! with the reasons itemized. Works on the tab as written (string/fret
//! assignments are already decided), so it measures the ACTUAL part —
//! pair with the fingering optimizer to bring the score down.

use tabmcp_model::Measure;

use crate::fingering::Tuning;

pub struct DifficultyReport {
    /// 1.0 (open-chord campfire) .. 10.0 (competition étude).
    pub score: f64,
    /// Human-readable contributing factors, hardest first.
    pub reasons: Vec<String>,
    pub notes_per_second: f64,
    pub max_stretch: u32,
    pub position_shifts_per_measure: f64,
}

struct TabOnset {
    tick: u64,
    string: u32,
    fret: u32,
    techniques: u32, // count of technique flags on this note
    bend: bool,
    tapping: bool,
    fast_pick: bool, // trem-picked or otherwise endurance-heavy
}

/// Analyze one track's range. `tempo_bpm` scales tick time into real time.
pub fn analyze(measures: &[Measure], _tuning: Tuning, tempo_bpm: u32) -> DifficultyReport {
    let mut onsets: Vec<TabOnset> = Vec::new();
    let mut chord_stretch_max = 0u32;
    for measure in measures {
        for beat in &measure.beats {
            let mut frets_in_chord: Vec<u32> = Vec::new();
            for voice in &beat.voices {
                for note in &voice.notes {
                    if note.tied {
                        continue;
                    }
                    if note.fret > 0 {
                        frets_in_chord.push(note.fret);
                    }
                    let e = &note.effects;
                    let techniques = [
                        e.vibrato,
                        e.slide,
                        e.hammer,
                        e.bend.is_some(),
                        e.harmonic.is_some(),
                        e.tapping,
                        e.dead_note,
                    ]
                    .iter()
                    .filter(|&&x| x)
                    .count() as u32;
                    onsets.push(TabOnset {
                        tick: beat.start_tick,
                        string: note.string,
                        fret: note.fret,
                        techniques,
                        bend: e.bend.is_some(),
                        tapping: e.tapping,
                        fast_pick: e.palm_mute || e.staccato,
                    });
                }
            }
            if let (Some(&lo), Some(&hi)) = (
                frets_in_chord.iter().min(),
                frets_in_chord.iter().max(),
            ) {
                chord_stretch_max = chord_stretch_max.max(hi - lo);
            }
        }
    }
    if onsets.is_empty() {
        return DifficultyReport {
            score: 1.0,
            reasons: vec!["empty range".into()],
            notes_per_second: 0.0,
            max_stretch: 0,
            position_shifts_per_measure: 0.0,
        };
    }
    onsets.sort_by_key(|o| o.tick);

    let ticks_total = onsets.last().unwrap().tick - onsets.first().unwrap().tick + 240;
    let seconds = ticks_total as f64 / 960.0 * 60.0 / tempo_bpm.max(1) as f64;
    let notes_per_second = onsets.len() as f64 / seconds.max(0.1);

    // Position shifts: consecutive fretted notes jumping >4 frets within a
    // beat's time (fast shift) — the classic "how does my hand get THERE".
    let mut fast_shifts = 0usize;
    let mut string_skips = 0usize;
    for pair in onsets.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let dt = b.tick - a.tick;
        if a.fret > 0 && b.fret > 0 && dt > 0 && dt <= 480 && a.fret.abs_diff(b.fret) > 4 {
            fast_shifts += 1;
        }
        if dt > 0 && dt <= 480 && a.string.abs_diff(b.string) >= 2 {
            string_skips += 1;
        }
    }
    let measures_n = measures.len().max(1) as f64;
    let shifts_per_measure = fast_shifts as f64 / measures_n;

    // Endurance: longest run of onsets at 16th-or-faster spacing.
    let mut run = 1usize;
    let mut longest_run = 1usize;
    for pair in onsets.windows(2) {
        let dt = pair[1].tick - pair[0].tick;
        if dt > 0 && dt <= 240 {
            run += 1;
            longest_run = longest_run.max(run);
        } else if dt > 0 {
            run = 1;
        }
    }

    // Finger fatigue: "can this be played for four minutes?" — longest
    // streak of consecutive measures at high notes/sec with no breather.
    let mut fatigue_streak = 0usize;
    {
        let mut current_streak = 0usize;
        for (i, measure) in measures.iter().enumerate() {
            let len = if i + 1 < measures.len() {
                measures[i + 1]
                    .start_tick
                    .saturating_sub(measure.start_tick)
            } else {
                3840
            }
            .max(1);
            let count = onsets
                .iter()
                .filter(|o| {
                    o.tick >= measure.start_tick && o.tick < measure.start_tick + len
                })
                .count();
            let seconds = len as f64 / 960.0 * 60.0 / tempo_bpm.max(1) as f64;
            if count as f64 / seconds.max(0.1) > 6.0 {
                current_streak += 1;
                fatigue_streak = fatigue_streak.max(current_streak);
            } else {
                current_streak = 0;
            }
        }
    }

    // Picking simulation: sweep shapes (3+ notes marching one string per hit
    // in one direction at speed) and zigzag crossings (rapid alternation of
    // string direction — inside-picking territory).
    let mut sweep_shapes = 0usize;
    let mut zigzag_crossings = 0usize;
    {
        let mut run_dir = 0i64;
        let mut run_len = 1usize;
        let mut last_dir = 0i64;
        for pair in onsets.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let dt = b.tick - a.tick;
            let dir = b.string as i64 - a.string as i64;
            if dt > 0 && dt <= 240 {
                if dir.abs() == 1 && (run_dir == 0 || dir == run_dir) {
                    run_dir = dir;
                    run_len += 1;
                    if run_len == 3 {
                        sweep_shapes += 1;
                    }
                } else {
                    run_dir = 0;
                    run_len = 1;
                }
                if dir != 0 && last_dir != 0 && dir.signum() != last_dir.signum() {
                    zigzag_crossings += 1;
                }
                if dir != 0 {
                    last_dir = dir;
                }
            } else {
                run_dir = 0;
                run_len = 1;
                last_dir = 0;
            }
        }
    }

    let technique_notes = onsets.iter().filter(|o| o.techniques > 0).count();
    let technique_share = technique_notes as f64 / onsets.len() as f64;
    let has_tapping = onsets.iter().any(|o| o.tapping);
    let bends = onsets.iter().filter(|o| o.bend).count();
    let picked_share =
        onsets.iter().filter(|o| o.fast_pick).count() as f64 / onsets.len() as f64;

    // Components on a 0..1 scale, then weighted into 1..10.
    let speed = (notes_per_second / 12.0).min(1.0); // 12 nps ~ 16ths at 180
    let stretch = (chord_stretch_max as f64 / 7.0).min(1.0);
    let shifts = (shifts_per_measure / 3.0).min(1.0);
    let skips = (string_skips as f64 / measures_n / 3.0).min(1.0);
    let endurance = (longest_run as f64 / 32.0).min(1.0); // 2 bars of 16ths
    let fatigue = (fatigue_streak as f64 / 8.0).min(1.0); // 8 bars no breather
    let technique = (technique_share * 1.5).min(1.0) + if has_tapping { 0.3 } else { 0.0 };

    let score = (1.0
        + speed * 4.0
        + stretch * 1.4
        + shifts * 1.4
        + skips * 0.8
        + endurance * 1.4
        + fatigue * 0.8
        + technique.min(1.0) * 1.0)
        .min(10.0);

    let mut reasons: Vec<(f64, String)> = Vec::new();
    if speed > 0.3 {
        reasons.push((
            speed,
            format!("{notes_per_second:.1} notes/sec sustained picking"),
        ));
    }
    if stretch > 0.4 {
        reasons.push((
            stretch,
            format!("{chord_stretch_max}-fret chord stretches"),
        ));
    }
    if shifts > 0.2 {
        reasons.push((
            shifts,
            format!("{fast_shifts} fast position shifts (>4 frets inside a beat)"),
        ));
    }
    if skips > 0.2 {
        reasons.push((skips, format!("{string_skips} quick string skips")));
    }
    if endurance > 0.4 {
        reasons.push((
            endurance,
            format!("{longest_run}-note continuous 16th run (endurance)"),
        ));
    }
    if technique_share > 0.15 {
        reasons.push((
            technique,
            format!(
                "{:.0}% of notes carry techniques{}{}",
                technique_share * 100.0,
                if bends > 0 { ", bends" } else { "" },
                if has_tapping { ", tapping" } else { "" }
            ),
        ));
    }
    if picked_share > 0.6 && notes_per_second > 6.0 {
        reasons.push((0.5, "tight palm-mute control at speed".into()));
    }
    if fatigue_streak >= 4 {
        reasons.push((
            fatigue,
            format!(
                "fatigue: {fatigue_streak} consecutive measures above 6 notes/sec with \
                 no breather - playable once, brutal for a full take"
            ),
        ));
    }
    if sweep_shapes > 0 {
        reasons.push((
            0.3,
            format!("{sweep_shapes} sweep-shaped runs (adjacent strings, one direction at speed)"),
        ));
    }
    if zigzag_crossings as f64 / measures_n > 2.0 {
        reasons.push((
            0.4,
            format!(
                "{zigzag_crossings} zigzag string crossings at 16th speed - awkward \
                 inside picking; consider refingering (tuxguitar_optimize_fingering)"
            ),
        ));
    }
    reasons.sort_by(|a, b| b.0.total_cmp(&a.0));

    DifficultyReport {
        score,
        reasons: reasons.into_iter().map(|(_, r)| r).collect(),
        notes_per_second,
        max_stretch: chord_stretch_max,
        position_shifts_per_measure: shifts_per_measure,
    }
}

pub fn describe(report: &DifficultyReport, label: &str) -> String {
    let level = match report.score {
        s if s < 3.0 => "beginner",
        s if s < 5.0 => "intermediate",
        s if s < 7.0 => "advanced",
        s if s < 8.5 => "expert",
        _ => "virtuoso",
    };
    let mut out = format!("{label}: difficulty {:.1}/10 ({level})\n", report.score);
    if report.reasons.is_empty() {
        out.push_str("  comfortable throughout - nothing flags as hard\n");
    }
    for reason in &report.reasons {
        out.push_str(&format!("  + {reason}\n"));
    }
    out
}

/// Silence the unused-field warnings until a caller needs the raw numbers.
pub fn _raw(report: &DifficultyReport) -> (f64, u32, f64) {
    (
        report.notes_per_second,
        report.max_stretch,
        report.position_shifts_per_measure,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tabmcp_model::{Beat, Duration, Note, NoteEffects, Tuplet, Voice};

    const STANDARD: &[(u32, u8)] = &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];

    fn measure_with(number: u32, step_ticks: u64, frets: &[u32]) -> Measure {
        let start = 960 * (1 + 4 * (number as u64 - 1));
        Measure {
            number,
            start_tick: start,
            key_signature: 0,
            beats: frets
                .iter()
                .enumerate()
                .map(|(i, &fret)| Beat {
                    start_tick: start + i as u64 * step_ticks,
                    voices: vec![Voice {
                        index: 0,
                        duration: Duration {
                            value: (3840 / step_ticks.max(60)) as u32,
                            dotted: false,
                            double_dotted: false,
                            tuplet: Tuplet { enters: 1, times: 1 },
                        },
                        is_rest: false,
                        notes: vec![Note {
                            string: 6,
                            fret,
                            velocity: 95,
                            tied: false,
                            effects: NoteEffects::default(),
                        }],
                    }],
                })
                .collect(),
        }
    }

    #[test]
    fn slow_open_riff_is_easy() {
        let m = vec![measure_with(1, 960, &[0, 3, 0, 5])];
        let report = analyze(&m, STANDARD, 90);
        assert!(report.score < 3.5, "{}", report.score);
    }

    #[test]
    fn fast_shifting_riff_is_hard() {
        // 16ths at 200 BPM leaping around the neck.
        let frets: Vec<u32> = (0..16).map(|i| if i % 2 == 0 { 2 } else { 14 }).collect();
        let m = vec![
            measure_with(1, 240, &frets),
            measure_with(2, 240, &frets),
        ];
        let report = analyze(&m, STANDARD, 200);
        assert!(report.score > 7.0, "{}", report.score);
        assert!(
            report.reasons.iter().any(|r| r.contains("position shifts")),
            "{:?}",
            report.reasons
        );
        let text = describe(&report, "T1");
        assert!(text.contains("difficulty"), "{text}");
    }

    fn measure_notes(number: u32, step_ticks: u64, notes: &[(u32, u32)]) -> Measure {
        let start = 960 * (1 + 4 * (number as u64 - 1));
        Measure {
            number,
            start_tick: start,
            key_signature: 0,
            beats: notes
                .iter()
                .enumerate()
                .map(|(i, &(string, fret))| Beat {
                    start_tick: start + i as u64 * step_ticks,
                    voices: vec![Voice {
                        index: 0,
                        duration: Duration {
                            value: (3840 / step_ticks.max(60)) as u32,
                            dotted: false,
                            double_dotted: false,
                            tuplet: Tuplet { enters: 1, times: 1 },
                        },
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
        }
    }

    #[test]
    fn relentless_sixteenths_flag_fatigue() {
        let frets: Vec<u32> = vec![0; 16];
        let m: Vec<Measure> = (1..=6).map(|n| measure_with(n, 240, &frets)).collect();
        let report = analyze(&m, STANDARD, 180);
        assert!(
            report.reasons.iter().any(|r| r.contains("fatigue")),
            "{:?}",
            report.reasons
        );
    }

    #[test]
    fn sweep_shapes_are_detected() {
        // Two 4-string sweeps: 6-5-4-3 then 3-4-5-6, 16ths.
        let notes: Vec<(u32, u32)> = vec![
            (6, 5), (5, 5), (4, 5), (3, 5),
            (3, 7), (4, 7), (5, 7), (6, 7),
        ];
        let m = vec![measure_notes(1, 240, &notes)];
        let report = analyze(&m, STANDARD, 140);
        assert!(
            report.reasons.iter().any(|r| r.contains("sweep")),
            "{:?}",
            report.reasons
        );
    }

    #[test]
    fn empty_range_reports_minimum() {
        let m = vec![Measure {
            number: 1,
            start_tick: 960,
            key_signature: 0,
            beats: vec![],
        }];
        assert_eq!(analyze(&m, STANDARD, 120).score, 1.0);
    }
}
