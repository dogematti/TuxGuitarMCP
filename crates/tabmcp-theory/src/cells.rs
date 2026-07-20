//! Rhythm-cell catalog: the named rhythmic building blocks complex riffs
//! are made of. A cell is a short pattern of onsets (with durations,
//! dots, tuplets) spanning a fixed tick length; riffs are cell sequences.
//! The constraint search uses this catalog as its rhythm alphabet, and
//! cells can be spelled onto pitches directly.

/// One onset inside a cell: (offset ticks, duration value, dotted,
/// tuplet enters, tuplet times). Duration value: 4 = quarter, 8 = eighth,
/// 16 = sixteenth, 32 = thirty-second.
pub type CellEvent = (u64, u32, bool, u32, u32);

pub struct RhythmCell {
    pub name: &'static str,
    /// What it sounds like / where it belongs.
    pub feel: &'static str,
    pub events: &'static [CellEvent],
    /// Total length in ticks (960 = one quarter-note beat).
    pub len: u64,
}

pub const CELLS: &[RhythmCell] = &[
    RhythmCell { name: "quarter", feel: "one solid hit - space and weight", events: &[(0, 4, false, 1, 1)], len: 960 },
    RhythmCell { name: "8ths", feel: "straight drive - punk/heavy backbone", events: &[(0, 8, false, 1, 1), (480, 8, false, 1, 1)], len: 960 },
    RhythmCell { name: "16ths", feel: "relentless chug - thrash/death engine", events: &[(0, 16, false, 1, 1), (240, 16, false, 1, 1), (480, 16, false, 1, 1), (720, 16, false, 1, 1)], len: 960 },
    RhythmCell { name: "gallop", feel: "8th + two 16ths - Maiden/power metal drive", events: &[(0, 8, false, 1, 1), (480, 16, false, 1, 1), (720, 16, false, 1, 1)], len: 960 },
    RhythmCell { name: "reverse-gallop", feel: "two 16ths + 8th - thrash's meaner sibling", events: &[(0, 16, false, 1, 1), (240, 16, false, 1, 1), (480, 8, false, 1, 1)], len: 960 },
    RhythmCell { name: "herta", feel: "32nd-pair flourish into 16th+8th - a stumble that lands", events: &[(0, 32, false, 1, 1), (120, 32, false, 1, 1), (240, 16, false, 1, 1), (480, 8, false, 1, 1)], len: 960 },
    RhythmCell { name: "offbeat-8ths", feel: "both hits off the beat - reggae-tinged push", events: &[(240, 8, false, 1, 1), (720, 8, false, 1, 1)], len: 960 },
    RhythmCell { name: "and-of-one", feel: "silence, then the push - classic syncopated stab", events: &[(480, 8, false, 1, 1)], len: 960 },
    RhythmCell { name: "tresillo", feel: "3+3+2 sixteenth groups over two beats - groove metal staple", events: &[(0, 16, false, 1, 1), (720, 16, false, 1, 1), (1440, 16, false, 1, 1), (1680, 16, false, 1, 1)], len: 1920 },
    RhythmCell { name: "hemiola", feel: "3 triplet quarters over 2 beats - the floor tilts", events: &[(0, 4, false, 3, 2), (640, 4, false, 3, 2), (1280, 4, false, 3, 2)], len: 1920 },
    RhythmCell { name: "triplet-8ths", feel: "rolling triplet feel over one beat", events: &[(0, 8, false, 3, 2), (320, 8, false, 3, 2), (640, 8, false, 3, 2)], len: 960 },
    RhythmCell { name: "quintuplet", feel: "5 over the beat - progressive vertigo", events: &[(0, 16, false, 5, 4), (192, 16, false, 5, 4), (384, 16, false, 5, 4), (576, 16, false, 5, 4), (768, 16, false, 5, 4)], len: 960 },
    RhythmCell { name: "dotted-8ths", feel: "dotted-8th chain - implied 3-over-4 polymeter", events: &[(0, 8, true, 1, 1), (720, 8, true, 1, 1), (1440, 8, true, 1, 1), (2160, 8, true, 1, 1)], len: 2880 },
    RhythmCell { name: "sixteenth-rest-start", feel: "16th rest then three 16ths - snaps off the beat", events: &[(240, 16, false, 1, 1), (480, 16, false, 1, 1), (720, 16, false, 1, 1)], len: 960 },
    RhythmCell { name: "rest-8th", feel: "half a beat of silence", events: &[], len: 480 },
    RhythmCell { name: "rest-quarter", feel: "a full beat of silence - let it breathe", events: &[], len: 960 },
];

pub fn cell(name: &str) -> Option<&'static RhythmCell> {
    let wanted = name.trim().to_ascii_lowercase();
    CELLS.iter().find(|c| c.name == wanted)
}

pub fn catalog() -> String {
    CELLS
        .iter()
        .map(|c| {
            format!(
                "  {} ({} onsets, {:.2} beats): {}",
                c.name,
                c.events.len(),
                c.len as f64 / 960.0,
                c.feel
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A spelled onset ready to become a beat: measure-relative offset plus
/// the duration fields.
#[derive(Clone)]
pub struct SpelledOnset {
    pub offset: u64,
    pub value: u32,
    pub dotted: bool,
    pub tuplet_enters: u32,
    pub tuplet_times: u32,
}

/// Lay a cell sequence across measures of the given lengths, cycling the
/// sequence until the range is full. A cell that does not fit in the
/// remaining space of a measure is replaced by silence to the barline
/// (cells never straddle barlines - that is what rebar is for).
pub fn spell(cell_names: &[&RhythmCell], measure_lens: &[u64]) -> Vec<Vec<SpelledOnset>> {
    let mut out = Vec::with_capacity(measure_lens.len());
    let mut cursor_cell = 0usize;
    for &len in measure_lens {
        let mut onsets = Vec::new();
        let mut pos = 0u64;
        while pos < len && !cell_names.is_empty() {
            let cell = cell_names[cursor_cell % cell_names.len()];
            if pos + cell.len > len {
                break; // silence to the barline
            }
            for &(offset, value, dotted, enters, times) in cell.events {
                onsets.push(SpelledOnset {
                    offset: pos + offset,
                    value,
                    dotted,
                    tuplet_enters: enters,
                    tuplet_times: times,
                });
            }
            pos += cell.len;
            cursor_cell += 1;
        }
        out.push(onsets);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lookup_and_shapes() {
        assert!(cell("gallop").is_some());
        assert!(cell("GALLOP ").is_some());
        assert!(cell("polka").is_none());
        // Every cell's events fit inside its declared length.
        for c in CELLS {
            for &(offset, ..) in c.events {
                assert!(offset < c.len, "{}: onset {offset} outside len {}", c.name, c.len);
            }
        }
        assert!(catalog().contains("tresillo"));
    }

    #[test]
    fn spell_fills_a_bar_and_cycles() {
        let seq = [cell("gallop").unwrap(), cell("reverse-gallop").unwrap()];
        let spelled = spell(&seq, &[3840, 3840]);
        assert_eq!(spelled.len(), 2);
        // 4 cells per 4/4 bar, 3 onsets each.
        assert_eq!(spelled[0].len(), 12);
        // First bar: gallop, reverse, gallop, reverse; second bar continues.
        assert_eq!(spelled[0][0].offset, 0);
        assert_eq!(spelled[0][3].offset, 960); // reverse-gallop starts beat 2
        // Cells never straddle the barline.
        assert!(spelled[1].iter().all(|o| o.offset < 3840));
    }

    #[test]
    fn spell_handles_odd_meters_with_silence_to_barline() {
        // 7/8 bar = 3360 ticks; tresillo (1920) + 8ths (960) = 2880, next
        // tresillo won't fit - remaining 480 stays silent.
        let seq = [cell("tresillo").unwrap(), cell("8ths").unwrap()];
        let spelled = spell(&seq, &[3360]);
        let max_offset = spelled[0].iter().map(|o| o.offset).max().unwrap();
        assert!(max_offset < 3360);
        assert_eq!(spelled[0].len(), 4 + 2);
    }
}
