//! MIDI pitch naming.

const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Pitch-class name for a MIDI pitch (sharps convention).
pub fn pitch_class_name(midi: u8) -> &'static str {
    NAMES[(midi % 12) as usize]
}

/// Scientific pitch notation for a MIDI pitch: 60 → "C4", 40 → "E2".
pub fn note_name(midi: u8) -> String {
    let octave = (midi as i32 / 12) - 1;
    format!("{}{}", pitch_class_name(midi), octave)
}

/// Parse scientific pitch notation ("A1", "F#3", "Bb2", "D♯4") to MIDI.
pub fn parse_note(name: &str) -> Option<u8> {
    let trimmed = name.trim();
    let mut chars = trimmed.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let mut pc: i32 = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let rest = rest.trim();
    let (accidental, octave_str) = match rest.chars().next() {
        Some('#') | Some('♯') => (1, &rest[rest.chars().next()?.len_utf8()..]),
        Some('b') | Some('♭') => (-1, &rest[rest.chars().next()?.len_utf8()..]),
        _ => (0, rest),
    };
    pc += accidental;
    let octave: i32 = octave_str.parse().ok()?;
    let midi = (octave + 1) * 12 + pc;
    u8::try_from(midi).ok()
}

/// Well-known tuning presets: name → open-string MIDI pitches, string 1
/// (highest-sounding) first — the order TuxGuitar uses.
pub const TUNING_PRESETS: &[(&str, &[u8])] = &[
    ("6-string standard", &[64, 59, 55, 50, 45, 40]), // E4 B3 G3 D3 A2 E2
    ("6-string drop D", &[64, 59, 55, 50, 45, 38]),   // ... D2
    ("6-string E-flat", &[63, 58, 54, 49, 44, 39]),   // Eb standard
    ("6-string drop C", &[62, 57, 53, 48, 43, 36]),   // D A F C G C
    ("7-string B standard", &[64, 59, 55, 50, 45, 40, 35]), // E4 ... B1
    ("7-string A standard", &[62, 57, 53, 48, 43, 38, 33]), // D4 A3 F3 C3 G2 D2 A1
    ("8-string F# standard", &[64, 59, 55, 50, 45, 40, 35, 30]),
    ("4-string bass", &[43, 38, 33, 28]),     // G2 D2 A1 E1
    ("5-string bass", &[43, 38, 33, 28, 23]), // ... B0
];

/// Look up a tuning preset by (case-insensitive) name.
pub fn tuning_preset(name: &str) -> Option<&'static [u8]> {
    let wanted = name.trim().to_ascii_lowercase();
    TUNING_PRESETS
        .iter()
        .find(|(preset, _)| preset.to_ascii_lowercase() == wanted)
        .map(|(_, pitches)| *pitches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_tuning_names() {
        // Standard guitar tuning, high to low: E4 B3 G3 D3 A2 E2.
        let tuning = [64u8, 59, 55, 50, 45, 40];
        let names: Vec<String> = tuning.iter().map(|&p| note_name(p)).collect();
        assert_eq!(names, vec!["E4", "B3", "G3", "D3", "A2", "E2"]);
    }

    #[test]
    fn middle_c() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(pitch_class_name(61), "C#");
    }

    #[test]
    fn parse_round_trips() {
        for midi in 21u8..=100 {
            assert_eq!(parse_note(&note_name(midi)), Some(midi));
        }
        assert_eq!(parse_note("F#3"), Some(54));
        assert_eq!(parse_note("Bb2"), Some(46));
        assert_eq!(parse_note("a1"), Some(33));
        assert_eq!(parse_note("H2"), None);
    }

    #[test]
    fn seven_string_a_standard_preset() {
        let pitches = tuning_preset("7-string A standard").expect("preset exists");
        let names: Vec<String> = pitches.iter().map(|&p| note_name(p)).collect();
        assert_eq!(names, vec!["D4", "A3", "F3", "C3", "G2", "D2", "A1"]);
    }
}
