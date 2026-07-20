//! Transition engine: what happens at section boundaries. Fills for the
//! drums in the last bar before a marker, buildups (velocity ramp +
//! densifying kicks), and band stops (everyone cuts, one hit remains).
//! Real arrangements live on these boundaries; butt-joined sections do not.

use tabmcp_model::{Beat, Duration, Measure, Note, NoteEffects, Tuplet, Voice};

use crate::generation::{DRUM_CRASH, DRUM_KICK, DRUM_SNARE};

const TOM_HIGH: u32 = 48;
const TOM_MID: u32 = 45;
const TOM_LOW: u32 = 43;

fn measure_len(measures: &[Measure], index: usize) -> u64 {
    if index + 1 < measures.len() {
        let len = measures[index + 1]
            .start_tick
            .saturating_sub(measures[index].start_tick);
        if len > 0 {
            return len;
        }
    }
    3840
}

fn drum_beat(start_tick: u64, offset_grid: u64, hits: Vec<(u32, u32)>) -> Beat {
    Beat {
        start_tick,
        voices: vec![Voice {
            index: 0,
            duration: Duration {
                value: if offset_grid % 480 == 0 { 8 } else { 16 },
                dotted: false,
                double_dotted: false,
                tuplet: Tuplet { enters: 1, times: 1 },
            },
            is_rest: false,
            notes: hits
                .into_iter()
                .map(|(key, velocity)| Note {
                    string: match key {
                        DRUM_KICK => 6,
                        DRUM_SNARE => 4,
                        DRUM_CRASH => 2,
                        _ => 3,
                    },
                    fret: key,
                    velocity,
                    tied: false,
                    effects: NoteEffects::default(),
                })
                .collect(),
        }],
    }
}

/// The transition kinds the engine writes.
#[derive(Clone, Copy, PartialEq)]
pub enum TransitionKind {
    /// Classic snare-tom fill over the last half of the bar.
    Fill,
    /// Full-bar buildup: snare 8ths -> 16ths with a velocity ramp.
    Buildup,
    /// Band stop: silence after beat 1's accent (drums keep one crash).
    Stop,
}

impl TransitionKind {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "fill" => Some(Self::Fill),
            "buildup" | "build" => Some(Self::Buildup),
            "stop" | "break" => Some(Self::Stop),
            _ => None,
        }
    }

    pub fn describe(&self) -> &'static str {
        match self {
            Self::Fill => "snare-tom fill over the back half",
            Self::Buildup => "snare buildup 8ths->16ths with rising velocity",
            Self::Stop => "band stop after the downbeat",
        }
    }
}

/// Rewrite ONE drum measure (the bar before a boundary) as a transition.
/// Keeps the first half of the original bar for Fill; Buildup and Stop
/// replace the whole bar. Returns the rewritten measure.
pub fn drum_transition(
    measures: &[Measure],
    index: usize,
    kind: TransitionKind,
) -> Measure {
    let source = &measures[index];
    let len = measure_len(measures, index);
    let start = source.start_tick;
    let mut out = Measure {
        number: source.number,
        start_tick: start,
        key_signature: source.key_signature,
        beats: Vec::new(),
    };
    match kind {
        TransitionKind::Fill => {
            // First half: original material.
            for beat in &source.beats {
                if beat.start_tick.saturating_sub(start) < len / 2 {
                    out.beats.push(beat.clone());
                }
            }
            // Back half: snare -> high tom -> mid tom -> low tom 16ths,
            // crash target lands on the NEXT bar (the section start).
            let half = len / 2;
            let sixteenth = 240u64;
            let lanes = [DRUM_SNARE, DRUM_SNARE, TOM_HIGH, TOM_HIGH, TOM_MID, TOM_MID, TOM_LOW, TOM_LOW];
            let slots = ((len - half) / sixteenth).min(8) as usize;
            for i in 0..slots {
                let offset = half + i as u64 * sixteenth;
                let velocity = 88 + (i as u32 * 4).min(32);
                out.beats
                    .push(drum_beat(start + offset, offset, vec![(lanes[i.min(7)], velocity)]));
            }
        }
        TransitionKind::Buildup => {
            // Kick holds quarters; snare 8ths for the first half, 16ths for
            // the second, velocity ramping 72 -> 120.
            let mut offset = 0u64;
            while offset < len {
                let progress = offset as f64 / len as f64;
                let velocity = (72.0 + progress * 48.0) as u32;
                let mut hits = Vec::new();
                if offset % 960 == 0 {
                    hits.push((DRUM_KICK, 100));
                }
                let grid = if progress < 0.5 { 480 } else { 240 };
                if offset % grid == 0 {
                    hits.push((DRUM_SNARE, velocity.min(127)));
                }
                if !hits.is_empty() {
                    out.beats.push(drum_beat(start + offset, offset, hits));
                }
                offset += 240;
            }
        }
        TransitionKind::Stop => {
            // One accented crash+kick on the downbeat, then silence.
            out.beats.push(drum_beat(
                start,
                0,
                vec![(DRUM_KICK, 118), (DRUM_CRASH, 118)],
            ));
        }
    }
    out.beats.sort_by_key(|b| b.start_tick);
    out
}

/// Rewrite ONE melodic measure for a Stop transition: keep onsets up to the
/// downbeat accent (with letRing on the last one), drop the rest. For
/// Buildup: keep the bar but ramp velocities upward. Fill leaves melodic
/// tracks untouched (returns None).
pub fn melodic_transition(
    measures: &[Measure],
    index: usize,
    kind: TransitionKind,
) -> Option<Measure> {
    let source = &measures[index];
    let len = measure_len(measures, index);
    let start = source.start_tick;
    match kind {
        TransitionKind::Fill => None,
        TransitionKind::Stop => {
            let mut out = Measure {
                number: source.number,
                start_tick: start,
                key_signature: source.key_signature,
                beats: Vec::new(),
            };
            // Keep whatever sounds within the first beat; let the last ring.
            for beat in &source.beats {
                if beat.start_tick.saturating_sub(start) < 960 {
                    out.beats.push(beat.clone());
                }
            }
            if let Some(last) = out.beats.last_mut() {
                for voice in &mut last.voices {
                    for note in &mut voice.notes {
                        note.velocity = note.velocity.max(110).min(127);
                        note.effects.let_ring = true;
                        note.effects.palm_mute = false;
                    }
                }
            }
            Some(out)
        }
        TransitionKind::Buildup => {
            let mut out = source.clone();
            for beat in &mut out.beats {
                let progress =
                    beat.start_tick.saturating_sub(start) as f64 / len as f64;
                for voice in &mut beat.voices {
                    for note in &mut voice.notes {
                        let ramped = 80.0 + progress * 45.0;
                        note.velocity = (ramped as u32).clamp(60, 127);
                    }
                }
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drum_measure(number: u32) -> Measure {
        let start = 960 * (1 + 4 * (number as u64 - 1));
        Measure {
            number,
            start_tick: start,
            key_signature: 0,
            beats: (0..8u64)
                .map(|j| drum_beat(start + j * 480, j * 480, vec![(DRUM_KICK, 100)]))
                .collect(),
        }
    }

    #[test]
    fn fill_keeps_front_half_and_ramps_toms() {
        let measures = vec![drum_measure(1), drum_measure(2)];
        let fill = drum_transition(&measures, 0, TransitionKind::Fill);
        // Front half: 4 original beats; back half: 8 sixteenth fill hits.
        assert_eq!(fill.beats.len(), 4 + 8);
        let last = fill.beats.last().unwrap();
        assert_eq!(last.voices[0].notes[0].fret, TOM_LOW);
        // Velocity ramps upward through the fill.
        let fill_hits: Vec<u32> = fill.beats[4..]
            .iter()
            .map(|b| b.voices[0].notes[0].velocity)
            .collect();
        assert!(fill_hits.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn buildup_ramps_velocity_and_densifies() {
        let measures = vec![drum_measure(1), drum_measure(2)];
        let build = drum_transition(&measures, 0, TransitionKind::Buildup);
        let snares: Vec<(u64, u32)> = build
            .beats
            .iter()
            .flat_map(|b| {
                let offset = b.start_tick - build.start_tick;
                b.voices[0]
                    .notes
                    .iter()
                    .filter(|n| n.fret == DRUM_SNARE)
                    .map(move |n| (offset, n.velocity))
                    .collect::<Vec<_>>()
            })
            .collect();
        // Denser in the second half than the first.
        let first_half = snares.iter().filter(|(o, _)| *o < 1920).count();
        let second_half = snares.iter().filter(|(o, _)| *o >= 1920).count();
        assert!(second_half > first_half, "{first_half} vs {second_half}");
        assert!(snares.last().unwrap().1 > snares.first().unwrap().1);
    }

    #[test]
    fn stop_leaves_one_accent() {
        let measures = vec![drum_measure(1), drum_measure(2)];
        let stop = drum_transition(&measures, 0, TransitionKind::Stop);
        assert_eq!(stop.beats.len(), 1);
        assert!(stop.beats[0]
            .voices[0]
            .notes
            .iter()
            .any(|n| n.fret == DRUM_CRASH));
    }

    #[test]
    fn melodic_stop_keeps_downbeat_with_let_ring() {
        let start = 960u64;
        let melodic = Measure {
            number: 1,
            start_tick: start,
            key_signature: 0,
            beats: (0..8u64)
                .map(|j| Beat {
                    start_tick: start + j * 480,
                    voices: vec![Voice {
                        index: 0,
                        duration: Duration {
                            value: 8,
                            dotted: false,
                            double_dotted: false,
                            tuplet: Tuplet { enters: 1, times: 1 },
                        },
                        is_rest: false,
                        notes: vec![Note {
                            string: 7,
                            fret: 0,
                            velocity: 95,
                            tied: false,
                            effects: NoteEffects {
                                palm_mute: true,
                                ..NoteEffects::default()
                            },
                        }],
                    }],
                })
                .collect(),
        };
        let measures = vec![melodic.clone(), melodic];
        let stopped = melodic_transition(&measures, 0, TransitionKind::Stop).unwrap();
        assert_eq!(stopped.beats.len(), 2); // offsets 0 and 480 only
        let last_note = &stopped.beats.last().unwrap().voices[0].notes[0];
        assert!(last_note.effects.let_ring);
        assert!(!last_note.effects.palm_mute);
        assert!(melodic_transition(&measures, 0, TransitionKind::Fill).is_none());
    }
}
