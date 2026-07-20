//! Riff transforms: mechanical variations of existing material.
//! Meter-aware (per-measure lengths derived from consecutive startTicks).

use tabmcp_model::Measure;

fn lengths(measures: &[Measure]) -> Vec<u64> {
    let mut out = Vec::with_capacity(measures.len());
    for i in 0..measures.len() {
        let len = if i + 1 < measures.len() {
            measures[i + 1]
                .start_tick
                .saturating_sub(measures[i].start_tick)
        } else {
            out.last().copied().unwrap_or(0)
        };
        out.push(if len > 0 { len } else { 3840 });
    }
    out
}

/// Rotate every measure's beats forward by `ticks` (wrapping) — rhythmic
/// displacement: the same riff, landing off the original grid.
pub fn displace(measures: &mut [Measure], ticks: u64) {
    let lens = lengths(measures);
    for (measure, len) in measures.iter_mut().zip(lens) {
        for beat in &mut measure.beats {
            let offset = beat.start_tick.saturating_sub(measure.start_tick);
            beat.start_tick = measure.start_tick + (offset + ticks) % len;
        }
        measure.beats.sort_by_key(|b| b.start_tick);
    }
}

/// Reverse the order of onsets across the whole range (pitch content walks
/// backward through the original rhythm slots).
pub fn retrograde(measures: &mut [Measure]) {
    let mut all_voices: Vec<Vec<tabmcp_model::Voice>> = measures
        .iter()
        .flat_map(|m| m.beats.iter().map(|b| b.voices.clone()))
        .collect();
    all_voices.reverse();
    let mut iter = all_voices.into_iter();
    for measure in measures.iter_mut() {
        for beat in &mut measure.beats {
            if let Some(voices) = iter.next() {
                beat.voices = voices;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tabmcp_model::{Beat, Duration, Note, NoteEffects, Tuplet, Voice};

    fn riff() -> Vec<Measure> {
        vec![Measure {
            number: 1,
            start_tick: 960,
            key_signature: 0,
            beats: (0..4u64)
                .map(|j| Beat {
                    start_tick: 960 + j * 480,
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
                            string: 6,
                            fret: j as u32,
                            velocity: 90,
                            tied: false,
                            effects: NoteEffects::default(),
                        }],
                    }],
                })
                .collect(),
        }]
    }

    #[test]
    fn displace_rotates_within_the_measure() {
        let mut m = riff();
        displace(&mut m, 480);
        let offsets: Vec<u64> = m[0].beats.iter().map(|b| b.start_tick - 960).collect();
        assert_eq!(offsets, vec![480, 960, 1440, 1920]);
        assert_eq!(m[0].beats.len(), 4);
    }

    #[test]
    fn retrograde_reverses_pitch_order() {
        let mut m = riff();
        retrograde(&mut m);
        let frets: Vec<u32> = m[0]
            .beats
            .iter()
            .map(|b| b.voices[0].notes[0].fret)
            .collect();
        assert_eq!(frets, vec![3, 2, 1, 0]);
    }
}
