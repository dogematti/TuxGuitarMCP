//! Ornament pass: style-idiomatic articulation over existing material.
//! Generators write clean notes; this decorates them the way a player
//! would - vibrato on held notes, slides into phrase starts, pinch
//! squeals on accent peaks, tremolo conversion for extreme styles.
//! Realism-aware: no bends on low wound frets, no vibrato on 16ths.

use tabmcp_model::{GraceEffect, HarmonicEffect, Measure, TremoloPickingEffect};

use crate::fingering::Tuning;

pub struct OrnamentReport {
    pub vibrato: usize,
    pub slides: usize,
    pub pinches: usize,
    pub grace: usize,
    pub tremolo: usize,
    pub notes_seen: usize,
}

/// Style flavor for the decoration choices.
#[derive(Clone, Copy, PartialEq)]
pub enum Flavor {
    /// Vibrato + slides + occasional pinch (default metal/rock).
    Standard,
    /// Everything above plus pinch on every heavy accent (groove/djent).
    Aggressive,
    /// Convert sustained notes to tremolo picking (black/death/surf).
    Tremolo,
}

impl Flavor {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "standard" | "default" | "" => Some(Self::Standard),
            "aggressive" | "metal" => Some(Self::Aggressive),
            "tremolo" | "extreme" => Some(Self::Tremolo),
            _ => None,
        }
    }
}

fn duration_ticks(value: u32, dotted: bool) -> u64 {
    let base = 3840u64 / value.max(1) as u64;
    if dotted { base + base / 2 } else { base }
}

/// Decorate a range in place. Deterministic: the same input produces the
/// same ornaments (position-hashed choices, no RNG).
pub fn decorate(measures: &mut [Measure], tuning: Tuning, flavor: Flavor) -> OrnamentReport {
    let open: std::collections::HashMap<u32, u8> =
        tuning.iter().copied().collect();
    let mut report = OrnamentReport {
        vibrato: 0,
        slides: 0,
        pinches: 0,
        grace: 0,
        tremolo: 0,
        notes_seen: 0,
    };
    // Phrase starts: first onset of each measure after a gap or bar line.
    for mi in 0..measures.len() {
        let measure_start = measures[mi].start_tick;
        let beat_count = measures[mi].beats.len();
        for bi in 0..beat_count {
            let (start_tick, gap_before) = {
                let beat = &measures[mi].beats[bi];
                let gap = if bi == 0 {
                    true
                } else {
                    let prev = &measures[mi].beats[bi - 1];
                    beat.start_tick.saturating_sub(prev.start_tick) >= 960
                };
                (beat.start_tick, gap)
            };
            let offset = start_tick.saturating_sub(measure_start);
            let beat = &mut measures[mi].beats[bi];
            for voice in &mut beat.voices {
                let held = duration_ticks(voice.duration.value, voice.duration.dotted) >= 1440;
                let long_note = duration_ticks(voice.duration.value, voice.duration.dotted) >= 960;
                for note in &mut voice.notes {
                    if note.tied {
                        continue;
                    }
                    report.notes_seen += 1;
                    let fretted_high = note.fret >= 5;
                    let heavy = note.velocity >= 108 || note.effects.heavy_accent;
                    let accent = note.effects.accent || heavy;
                    let is_low_string = open
                        .get(&note.string)
                        .map(|&o| o < 45)
                        .unwrap_or(false);

                    match flavor {
                        Flavor::Tremolo => {
                            // Sustained notes become tremolo picking.
                            if long_note && note.effects.tremolo_picking.is_none() {
                                note.effects.tremolo_picking =
                                    Some(TremoloPickingEffect { speed: 16 });
                                note.effects.palm_mute = false;
                                report.tremolo += 1;
                                continue;
                            }
                        }
                        _ => {
                            // Vibrato on held, fretted, un-muted notes.
                            if held
                                && fretted_high
                                && !note.effects.palm_mute
                                && !note.effects.vibrato
                            {
                                note.effects.vibrato = true;
                                report.vibrato += 1;
                            }
                        }
                    }

                    // Slide into phrase starts on fretted melodic notes
                    // (never on open strings; never on the very first beat
                    // of the piece where there is nothing to slide from).
                    if gap_before
                        && offset > 0
                        && note.fret >= 3
                        && !note.effects.slide
                        && !is_low_string
                    {
                        note.effects.slide = true;
                        report.slides += 1;
                    }

                    // Pinch squeals on accent peaks (low-string chug
                    // accents in aggressive flavor; position-hashed so not
                    // every accent squeals).
                    let wants_pinch = match flavor {
                        Flavor::Aggressive => accent && is_low_string,
                        Flavor::Standard => heavy && is_low_string,
                        Flavor::Tremolo => false,
                    };
                    if wants_pinch
                        && note.effects.harmonic.is_none()
                        && (start_tick / 240) % 4 == 0
                    {
                        note.effects.harmonic = Some(HarmonicEffect {
                            kind: "pinch".into(),
                            data: None,
                        });
                        report.pinches += 1;
                    }

                    // Grace hammer into the first melodic note of a measure
                    // when it sits mid-neck (classic entry ornament) -
                    // sparse: only every other measure by position hash.
                    if offset == 0
                        && note.fret >= 5
                        && !is_low_string
                        && note.effects.grace.is_none()
                        && flavor != Flavor::Tremolo
                        && (measure_start / 3840) % 2 == 1
                    {
                        note.effects.grace = Some(GraceEffect::default());
                        report.grace += 1;
                    }
                }
            }
        }
    }
    report
}

pub fn describe(report: &OrnamentReport) -> String {
    format!(
        "Ornamented {} notes: {} vibrato, {} slides, {} pinch harmonics, \
         {} grace notes, {} tremolo conversions",
        report.notes_seen,
        report.vibrato,
        report.slides,
        report.pinches,
        report.grace,
        report.tremolo
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tabmcp_model::{Beat, Duration, Note, NoteEffects, Tuplet, Voice};

    const SEVEN: &[(u32, u8)] = &[
        (1, 62),
        (2, 57),
        (3, 53),
        (4, 48),
        (5, 43),
        (6, 38),
        (7, 33),
    ];

    fn measure(number: u32, entries: &[(u64, u32, u32, u32, u32)]) -> Measure {
        // (offset, duration value, string, fret, velocity)
        let start = 960 * (1 + 4 * (number as u64 - 1));
        Measure {
            number,
            start_tick: start,
            key_signature: 0,
            beats: entries
                .iter()
                .map(|&(offset, value, string, fret, velocity)| Beat {
                    start_tick: start + offset,
                    voices: vec![Voice {
                        index: 0,
                        duration: Duration {
                            value,
                            dotted: false,
                            double_dotted: false,
                            tuplet: Tuplet { enters: 1, times: 1 },
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
    fn vibrato_on_held_notes_only() {
        // Held half note on fret 7 (string 3) + fast 16ths.
        let mut m = vec![measure(
            1,
            &[(0, 2, 3, 7, 95), (1920, 16, 3, 7, 95), (2160, 16, 3, 8, 95)],
        )];
        let report = decorate(&mut m, SEVEN, Flavor::Standard);
        assert_eq!(report.vibrato, 1);
        assert!(m[0].beats[0].voices[0].notes[0].effects.vibrato);
        assert!(!m[0].beats[1].voices[0].notes[0].effects.vibrato);
    }

    #[test]
    fn tremolo_flavor_converts_sustains() {
        let mut m = vec![measure(1, &[(0, 4, 2, 5, 95), (960, 4, 2, 8, 95)])];
        let report = decorate(&mut m, SEVEN, Flavor::Tremolo);
        assert_eq!(report.tremolo, 2);
        assert!(m[0].beats[0].voices[0].notes[0]
            .effects
            .tremolo_picking
            .is_some());
    }

    #[test]
    fn pinch_lands_on_low_string_accents() {
        // Heavy accents on the low A string at aligned grid slots.
        let mut m = vec![measure(
            1,
            &[(0, 8, 7, 0, 115), (480, 8, 7, 0, 90), (960, 8, 7, 0, 115)],
        )];
        let report = decorate(&mut m, SEVEN, Flavor::Aggressive);
        assert!(report.pinches >= 1, "{}", describe(&report));
        // The soft chug got nothing.
        assert!(m[0].beats[1].voices[0].notes[0].effects.harmonic.is_none());
    }

    #[test]
    fn deterministic() {
        let build = || {
            let mut m = vec![
                measure(1, &[(0, 2, 3, 7, 110), (1920, 4, 2, 9, 95)]),
                measure(2, &[(0, 4, 2, 7, 95), (1920, 2, 3, 5, 112)]),
            ];
            decorate(&mut m, SEVEN, Flavor::Standard);
            format!("{m:?}")
        };
        assert_eq!(build(), build());
    }
}
