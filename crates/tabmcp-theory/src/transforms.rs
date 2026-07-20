//! Riff transforms: mechanical variations of existing material.
//! Meter-aware (per-measure lengths derived from consecutive startTicks).

use std::collections::HashMap;

use tabmcp_model::{Beat, Duration, Measure, Note, NoteEffects, Tuplet, Voice};

use crate::fingering::Tuning;

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

fn note_pitch(open: &HashMap<u32, u8>, note: &Note) -> Option<i16> {
    open.get(&note.string)
        .map(|&o| o as i16 + note.fret as i16)
}

/// Refit a target pitch onto the tuning: prefer the original string, else the
/// string giving the lowest playable fret; octave-adjust into range as a last
/// resort. None only when the tuning can't reach the pitch class at all.
fn refit(open: &HashMap<u32, u8>, prefer_string: u32, pitch: i16) -> Option<(u32, u32)> {
    for candidate in [pitch, pitch - 12, pitch + 12, pitch - 24, pitch + 24] {
        if let Some(&o) = open.get(&prefer_string) {
            let fret = candidate - o as i16;
            if (0..=24).contains(&fret) {
                return Some((prefer_string, fret as u32));
            }
        }
        let best = open
            .iter()
            .filter_map(|(&s, &o)| {
                let fret = candidate - o as i16;
                (0..=24).contains(&fret).then_some((s, fret as u32))
            })
            .min_by_key(|&(_, fret)| fret);
        if best.is_some() {
            return best;
        }
    }
    None
}

fn map_pitches(measures: &mut [Measure], tuning: Tuning, f: impl Fn(i16) -> i16) {
    let open: HashMap<u32, u8> = tuning.iter().copied().collect();
    for measure in measures.iter_mut() {
        for beat in &mut measure.beats {
            for voice in &mut beat.voices {
                for note in &mut voice.notes {
                    if let Some(pitch) = note_pitch(&open, note) {
                        if let Some((string, fret)) = refit(&open, note.string, f(pitch)) {
                            note.string = string;
                            note.fret = fret;
                        }
                    }
                }
            }
        }
    }
}

/// Chromatic pitch inversion around the first sounding note (the axis keeps
/// its place; everything else mirrors — rising lines fall and vice versa).
pub fn invert(measures: &mut [Measure], tuning: Tuning) {
    let open: HashMap<u32, u8> = tuning.iter().copied().collect();
    let axis = measures
        .iter()
        .flat_map(|m| &m.beats)
        .flat_map(|b| &b.voices)
        .flat_map(|v| &v.notes)
        .find(|n| !n.tied)
        .and_then(|n| note_pitch(&open, n));
    let Some(axis) = axis else { return };
    map_pitches(measures, tuning, |p| 2 * axis - p);
}

/// Shift everything by whole octaves (negative = down). Pitches that fall off
/// the fretboard are octave-corrected back into range by the refitter.
pub fn octave_shift(measures: &mut [Measure], tuning: Tuning, octaves: i32) {
    map_pitches(measures, tuning, move |p| p + (12 * octaves) as i16);
}

fn scale_duration(duration: &Duration, factor_up: bool) -> Duration {
    let value = if factor_up {
        (duration.value / 2).max(1) // longer note (8th -> quarter)
    } else {
        (duration.value * 2).min(64) // shorter note (8th -> 16th)
    };
    Duration { value, ..duration.clone() }
}

fn collect_range(measures: &[Measure]) -> (Vec<u64>, Vec<(u64, Vec<Voice>)>) {
    let lens = lengths(measures);
    let mut cum = 0u64;
    let mut starts = Vec::with_capacity(measures.len());
    for len in &lens {
        starts.push(cum);
        cum += len;
    }
    let mut beats = Vec::new();
    for (measure, &start) in measures.iter().zip(&starts) {
        for beat in &measure.beats {
            let offset = beat.start_tick.saturating_sub(measure.start_tick);
            beats.push((start + offset, beat.voices.clone()));
        }
    }
    (lens, beats)
}

fn redistribute(measures: &mut [Measure], lens: &[u64], beats: Vec<(u64, Vec<Voice>)>) {
    let mut cum = 0u64;
    let mut bounds = Vec::with_capacity(measures.len());
    for len in lens {
        bounds.push((cum, cum + len));
        cum += len;
    }
    for measure in measures.iter_mut() {
        measure.beats.clear();
    }
    for (offset, voices) in beats {
        if let Some(index) = bounds
            .iter()
            .position(|&(lo, hi)| offset >= lo && offset < hi)
        {
            let start_tick = measures[index].start_tick + (offset - bounds[index].0);
            measures[index].beats.push(Beat { start_tick, voices });
        }
    }
    for measure in measures.iter_mut() {
        measure.beats.sort_by_key(|b| b.start_tick);
    }
}

/// Rhythmic augmentation: every duration doubles, the material stretches to
/// twice the length. Returns 2x the measures (the new tail continues the
/// source's meter); write it over a range twice the original size.
pub fn augment(measures: &[Measure]) -> Vec<Measure> {
    let (lens, beats) = collect_range(measures);
    let n = measures.len();
    let mut out: Vec<Measure> = Vec::with_capacity(n * 2);
    let mut next_start = measures[0].start_tick;
    for i in 0..n * 2 {
        let template = &measures[i % n];
        out.push(Measure {
            number: template.number,
            start_tick: next_start,
            key_signature: template.key_signature,
            beats: Vec::new(),
        });
        next_start += lens[i % n];
    }
    let doubled_lens: Vec<u64> = lens.iter().chain(lens.iter()).copied().collect();
    let stretched = beats
        .into_iter()
        .map(|(offset, voices)| {
            let voices = voices
                .into_iter()
                .map(|mut v| {
                    v.duration = scale_duration(&v.duration, true);
                    v
                })
                .collect();
            (offset * 2, voices)
        })
        .collect();
    redistribute(&mut out, &doubled_lens, stretched);
    out
}

/// Rhythmic diminution: every duration halves, the material compresses into
/// the first half of the range (the rest empties — repeat or fill it).
pub fn diminish(measures: &mut [Measure]) {
    let (lens, beats) = collect_range(measures);
    let compressed = beats
        .into_iter()
        .map(|(offset, voices)| {
            let voices = voices
                .into_iter()
                .map(|mut v| {
                    v.duration = scale_duration(&v.duration, false);
                    v
                })
                .collect();
            (offset / 2, voices)
        })
        .collect();
    redistribute(measures, &lens, compressed);
}

/// Fill every empty grid slot with a palm-muted pedal tone (the classic
/// metal device: riff notes ride over a relentless low chug).
pub fn pedal_fill(measures: &mut [Measure], string: u32, fret: u32, grid: u64) {
    let grid = grid.max(120);
    let lens = lengths(measures);
    let velocities: Vec<u32> = measures
        .iter()
        .flat_map(|m| &m.beats)
        .flat_map(|b| &b.voices)
        .flat_map(|v| &v.notes)
        .map(|n| n.velocity)
        .collect();
    let mean = if velocities.is_empty() {
        95.0
    } else {
        velocities.iter().sum::<u32>() as f64 / velocities.len() as f64
    };
    let pedal_velocity = ((mean * 0.85) as u32).clamp(20, 127);
    let value = ((3840 / grid) as u32).clamp(1, 64);
    for (measure, len) in measures.iter_mut().zip(lens) {
        let existing: Vec<u64> = measure
            .beats
            .iter()
            .map(|b| b.start_tick.saturating_sub(measure.start_tick))
            .collect();
        let mut slot = 0u64;
        while slot + grid <= len {
            let occupied = existing
                .iter()
                .any(|&o| o.abs_diff(slot) < grid / 2);
            if !occupied {
                measure.beats.push(Beat {
                    start_tick: measure.start_tick + slot,
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
                            velocity: pedal_velocity,
                            tied: false,
                            effects: NoteEffects {
                                palm_mute: true,
                                ..NoteEffects::default()
                            },
                        }],
                    }],
                });
            }
            slot += grid;
        }
        measure.beats.sort_by_key(|b| b.start_tick);
    }
}

/// Implied polymeter / regrouping: accent the start of each group in a
/// repeating pattern like 3+3+2 (units of `unit` ticks, cycling through the
/// range without restarting at barlines — that's what makes it polymetric).
pub fn accent_group(measures: &mut [Measure], groups: &[u32], unit: u64) {
    if groups.is_empty() || unit == 0 {
        return;
    }
    let mut group_starts = Vec::with_capacity(groups.len());
    let mut cum = 0u64;
    for &g in groups {
        group_starts.push(cum * unit);
        cum += g as u64;
    }
    let cycle = cum * unit;
    if cycle == 0 {
        return;
    }
    let range_start = measures[0].start_tick;
    for measure in measures.iter_mut() {
        for beat in &mut measure.beats {
            let rel = beat.start_tick.saturating_sub(range_start) % cycle;
            let accented = group_starts.contains(&rel);
            for voice in &mut beat.voices {
                for note in &mut voice.notes {
                    if accented {
                        note.velocity = (note.velocity + 18).min(127);
                        note.effects.accent = true;
                    }
                }
            }
        }
    }
}

/// Dynamics swap: palm-muted notes become accented open stabs and vice versa
/// — turns a chug line into its call-and-response counterpart.
pub fn swap_dynamics(measures: &mut [Measure]) {
    for measure in measures.iter_mut() {
        for beat in &mut measure.beats {
            for voice in &mut beat.voices {
                for note in &mut voice.notes {
                    let was_muted = note.effects.palm_mute;
                    let was_accented = note.effects.accent || note.effects.heavy_accent;
                    note.effects.palm_mute = was_accented;
                    note.effects.accent = was_muted;
                    note.effects.heavy_accent = false;
                    if was_muted {
                        note.velocity = (note.velocity + 12).min(127);
                    } else if was_accented {
                        note.velocity = note.velocity.saturating_sub(12).max(20);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    const STANDARD: &[(u32, u8)] = &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];

    fn pitches(m: &[Measure]) -> Vec<i16> {
        let open: HashMap<u32, u8> = STANDARD.iter().copied().collect();
        m.iter()
            .flat_map(|x| &x.beats)
            .flat_map(|b| &b.voices)
            .flat_map(|v| &v.notes)
            .map(|n| note_pitch(&open, n).unwrap())
            .collect()
    }

    #[test]
    fn invert_mirrors_around_first_note() {
        let mut m = riff(); // frets 0,1,2,3 on string 6 -> pitches 40,41,42,43
        invert(&mut m, STANDARD);
        assert_eq!(pitches(&m), vec![40, 51, 50, 49]); // 39,38,37 refit up an octave
    }

    #[test]
    fn octave_shift_up_moves_all_pitches() {
        let mut m = riff();
        octave_shift(&mut m, STANDARD, 1);
        assert_eq!(pitches(&m), vec![52, 53, 54, 55]);
    }

    #[test]
    fn augment_doubles_length_and_durations() {
        let out = augment(&riff());
        assert_eq!(out.len(), 2);
        let offsets: Vec<u64> = out
            .iter()
            .flat_map(|m| m.beats.iter().map(move |b| b.start_tick - m.start_tick))
            .collect();
        assert_eq!(offsets, vec![0, 960, 1920, 2880]);
        assert_eq!(out[0].beats[0].voices[0].duration.value, 4); // 8th -> quarter
    }

    #[test]
    fn diminish_compresses_into_first_half() {
        let mut m = riff();
        diminish(&mut m);
        let offsets: Vec<u64> = m[0].beats.iter().map(|b| b.start_tick - 960).collect();
        assert_eq!(offsets, vec![0, 240, 480, 720]);
        assert_eq!(m[0].beats[0].voices[0].duration.value, 16); // 8th -> 16th
        assert!(offsets.iter().all(|&o| o < 1920));
    }

    #[test]
    fn pedal_fill_adds_muted_chugs_in_gaps() {
        let mut m = riff(); // onsets every 480 ticks
        pedal_fill(&mut m, 6, 0, 240);
        // 4 riff notes + 12 free 16th slots (the riff covers only half the bar)
        assert_eq!(m[0].beats.len(), 16);
        let pedal = m[0]
            .beats
            .iter()
            .find(|b| b.start_tick == 960 + 240)
            .unwrap();
        let note = &pedal.voices[0].notes[0];
        assert!(note.effects.palm_mute);
        assert_eq!(note.fret, 0);
    }

    #[test]
    fn accent_group_marks_polymetric_group_starts() {
        let mut m = riff(); // onsets at 0,480,960,1440 within the measure
        accent_group(&mut m, &[3, 3, 2], 240); // group starts at 0, 720, 1440
        let accents: Vec<bool> = m[0]
            .beats
            .iter()
            .map(|b| b.voices[0].notes[0].effects.accent)
            .collect();
        assert_eq!(accents, vec![true, false, false, true]);
        assert_eq!(m[0].beats[0].voices[0].notes[0].velocity, 108);
    }

    #[test]
    fn swap_dynamics_flips_mutes_and_accents() {
        let mut m = riff();
        m[0].beats[0].voices[0].notes[0].effects.palm_mute = true;
        m[0].beats[1].voices[0].notes[0].effects.accent = true;
        swap_dynamics(&mut m);
        assert!(m[0].beats[0].voices[0].notes[0].effects.accent);
        assert!(!m[0].beats[0].voices[0].notes[0].effects.palm_mute);
        assert!(m[0].beats[1].voices[0].notes[0].effects.palm_mute);
        assert!(!m[0].beats[1].voices[0].notes[0].effects.accent);
    }
}
