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
}
