//! Guitar realism checker: catches things a human hand cannot (or should
//! not) do — impossible stretches, duplicate strings in a chord, frets past
//! the neck, awkward bends and open-string mixes. The tab compiles; this
//! asks whether it is guitar music.

use tabmcp_model::Measure;

pub struct RealismIssue {
    pub measure: u32,
    /// "impossible" or "awkward".
    pub severity: &'static str,
    pub text: String,
}

pub struct RealismReport {
    pub issues: Vec<RealismIssue>,
    pub notes_checked: usize,
}

pub fn check(measures: &[Measure], max_fret: u32) -> RealismReport {
    let mut issues = Vec::new();
    let mut notes_checked = 0usize;
    for measure in measures {
        for beat in &measure.beats {
            let mut strings_seen: Vec<u32> = Vec::new();
            let mut fretted: Vec<u32> = Vec::new();
            let mut has_open = false;
            for voice in &beat.voices {
                for note in &voice.notes {
                    notes_checked += 1;
                    if note.fret > max_fret {
                        issues.push(RealismIssue {
                            measure: measure.number,
                            severity: "impossible",
                            text: format!(
                                "fret {} is past the neck (track has {max_fret} frets)",
                                note.fret
                            ),
                        });
                    }
                    if strings_seen.contains(&note.string) {
                        issues.push(RealismIssue {
                            measure: measure.number,
                            severity: "impossible",
                            text: format!(
                                "string {} sounded twice in one beat - one string, one note",
                                note.string
                            ),
                        });
                    }
                    strings_seen.push(note.string);
                    if note.fret == 0 {
                        has_open = true;
                    } else {
                        fretted.push(note.fret);
                    }
                    if let Some(bend) = &note.effects.bend {
                        let max_semitones = bend.points.iter().map(|p| p.value).max().unwrap_or(0);
                        if max_semitones > 4 {
                            issues.push(RealismIssue {
                                measure: measure.number,
                                severity: "impossible",
                                text: format!(
                                    "bend of {max_semitones} semitones - strings break \
                                     around 2 full tones"
                                ),
                            });
                        } else if note.string >= 5 && note.fret <= 3 && note.fret > 0 {
                            issues.push(RealismIssue {
                                measure: measure.number,
                                severity: "awkward",
                                text: format!(
                                    "bend on wound string {} at fret {} - very stiff; move \
                                     the bend up the neck or to a thinner string",
                                    note.string, note.fret
                                ),
                            });
                        }
                    }
                }
            }
            if let (Some(&lo), Some(&hi)) = (fretted.iter().min(), fretted.iter().max()) {
                let span = hi - lo;
                if span > 6 {
                    issues.push(RealismIssue {
                        measure: measure.number,
                        severity: "impossible",
                        text: format!(
                            "chord spans {span} frets ({lo}-{hi}) - a hand covers about 5"
                        ),
                    });
                } else if span > 4 {
                    issues.push(RealismIssue {
                        measure: measure.number,
                        severity: "awkward",
                        text: format!("chord spans {span} frets ({lo}-{hi}) - big stretch"),
                    });
                }
                if has_open && lo >= 12 {
                    issues.push(RealismIssue {
                        measure: measure.number,
                        severity: "awkward",
                        text: format!(
                            "open string inside a fret-{lo}+ chord - rings against the \
                             high position; intentional only with letRing"
                        ),
                    });
                }
            }
        }
    }
    RealismReport {
        issues,
        notes_checked,
    }
}

pub fn describe(report: &RealismReport, label: &str) -> String {
    let impossible = report
        .issues
        .iter()
        .filter(|i| i.severity == "impossible")
        .count();
    let awkward = report.issues.len() - impossible;
    let mut out = format!(
        "REALISM CHECK {label}: {} notes checked - {impossible} impossible, {awkward} awkward\n",
        report.notes_checked
    );
    if report.issues.is_empty() {
        out.push_str("  clean - everything here is playable guitar\n");
        return out;
    }
    for issue in &report.issues {
        out.push_str(&format!(
            "  m{} [{}] {}\n",
            issue.measure, issue.severity, issue.text
        ));
    }
    out.push_str("  Fix impossible items first - they will not survive a human take.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tabmcp_model::{Beat, BendEffect, BendPoint, Duration, Note, NoteEffects, Tuplet, Voice};

    fn beat_with(start_tick: u64, notes: Vec<Note>) -> Beat {
        Beat {
            start_tick,
            voices: vec![Voice {
                index: 0,
                duration: Duration {
                    value: 4,
                    dotted: false,
                    double_dotted: false,
                    tuplet: Tuplet { enters: 1, times: 1 },
                },
                is_rest: false,
                notes,
            }],
        }
    }

    fn note(string: u32, fret: u32) -> Note {
        Note {
            string,
            fret,
            velocity: 95,
            tied: false,
            effects: NoteEffects::default(),
        }
    }

    #[test]
    fn flags_wide_chord_and_duplicate_string() {
        let measures = vec![Measure {
            number: 1,
            start_tick: 960,
            key_signature: 0,
            beats: vec![beat_with(
                960,
                vec![note(6, 1), note(5, 9), note(5, 3)],
            )],
        }];
        let report = check(&measures, 24);
        let text = describe(&report, "T1");
        assert!(text.contains("spans 8 frets"), "{text}");
        assert!(text.contains("sounded twice"), "{text}");
    }

    #[test]
    fn flags_monster_bend_and_stiff_low_bend() {
        let mut monster = note(1, 15);
        monster.effects.bend = Some(BendEffect {
            points: vec![
                BendPoint { position: 0, value: 0 },
                BendPoint { position: 12, value: 6 },
            ],
        });
        let mut stiff = note(6, 1);
        stiff.effects.bend = Some(BendEffect {
            points: vec![
                BendPoint { position: 0, value: 0 },
                BendPoint { position: 12, value: 2 },
            ],
        });
        let measures = vec![Measure {
            number: 2,
            start_tick: 960,
            key_signature: 0,
            beats: vec![beat_with(960, vec![monster]), beat_with(1920, vec![stiff])],
        }];
        let report = check(&measures, 24);
        let text = describe(&report, "T1");
        assert!(text.contains("strings break"), "{text}");
        assert!(text.contains("wound string"), "{text}");
    }

    #[test]
    fn clean_riff_reports_clean() {
        let measures = vec![Measure {
            number: 1,
            start_tick: 960,
            key_signature: 0,
            beats: vec![beat_with(960, vec![note(6, 0), note(5, 2)])],
        }];
        let report = check(&measures, 24);
        assert!(report.issues.is_empty());
        assert!(describe(&report, "T1").contains("clean"));
    }
}
