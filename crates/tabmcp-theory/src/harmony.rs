//! Harmony planner: progressions with functional roles and a
//! voice-leading check. Battle evidence: every AI entry pedals one root
//! for a whole song - nothing PLANNED harmonic motion. This module turns
//! "give me a progression for the chorus" into concrete power-chord
//! voicings with validated voice leading.

use crate::analysis::SCALES;
use crate::pitch::parse_note;

pub struct PlannedChord {
    /// Roman numeral in the requested scale (lowercase = minor quality).
    pub numeral: String,
    /// Root pitch class 0..11.
    pub root_pc: u8,
    /// Chord quality: "5" (power), "m", "maj", "dim".
    pub quality: &'static str,
}

pub struct HarmonyPlan {
    pub chords: Vec<PlannedChord>,
    /// Voice-leading notes: root motion per step, smoothness verdicts.
    pub notes: Vec<String>,
}

/// Named progression recipes per mood. Numerals are scale degrees
/// (1-based); quality derives from the scale.
const RECIPES: &[(&str, &[usize], &str)] = &[
    ("dark", &[1, 2, 1, 6], "i-bII pedal menace with a bVI lift at the tail"),
    ("epic", &[1, 6, 7, 1], "i-bVI-bVII-i - the metal anthem cadence"),
    ("driving", &[1, 7, 6, 7], "i-bVII-bVI-bVII - momentum without leaving home"),
    ("sad", &[1, 4, 6, 5], "i-iv-bVI-v - the mournful lap"),
    ("triumphant", &[1, 4, 5, 1], "i-iv-V-i - tension and release (raise the V!)"),
    ("unresolved", &[1, 2, 4, 2], "i-bII-iv-bII - circles without cadence"),
    ("lift", &[6, 7, 1, 1], "bVI-bVII-i - approach the root from above (chorus lift)"),
];

pub fn moods() -> Vec<&'static str> {
    RECIPES.iter().map(|(name, _, _)| *name).collect()
}

fn quality_of(scale_steps: &[u8], degree_index: usize) -> &'static str {
    // Triad from stacked scale thirds; classify by third+fifth.
    let n = scale_steps.len();
    let root = scale_steps[degree_index % n] as i16;
    let third = scale_steps[(degree_index + 2) % n] as i16;
    let fifth = scale_steps[(degree_index + 4) % n] as i16;
    let third_iv = ((third - root).rem_euclid(12)) as u8;
    let fifth_iv = ((fifth - root).rem_euclid(12)) as u8;
    match (third_iv, fifth_iv) {
        (4, 7) => "maj",
        (3, 7) => "m",
        (3, 6) => "dim",
        _ => "5",
    }
}

fn numeral(degree: usize, quality: &str) -> String {
    let base = match degree {
        1 => "I",
        2 => "II",
        3 => "III",
        4 => "IV",
        5 => "V",
        6 => "VI",
        7 => "VII",
        _ => "?",
    };
    match quality {
        "m" | "dim" => base.to_lowercase() + if quality == "dim" { "\u{00b0}" } else { "" },
        _ => base.to_string(),
    }
}

/// Plan a progression: either a named mood recipe or explicit degrees
/// like "1-6-7-1". Scale spec: "A natural minor" etc.
pub fn plan(scale_spec: &str, request: &str) -> Result<HarmonyPlan, String> {
    let (root_name, scale_name) = scale_spec
        .trim()
        .split_once(' ')
        .ok_or_else(|| "scale must be '<root> <name>', e.g. 'A natural minor'".to_string())?;
    let root_pc = parse_note(&format!("{root_name}4"))
        .map(|p| p % 12)
        .ok_or_else(|| format!("unknown root '{root_name}'"))?;
    let steps = SCALES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(scale_name.trim()))
        .map(|(_, s)| *s)
        .ok_or_else(|| format!("unknown scale '{}'", scale_name.trim()))?;
    if steps.len() < 7 {
        return Err(
            "scale too small for triadic planning - use a 7-note scale (natural/harmonic/\
             melodic minor, phrygian dominant, ...)"
                .into(),
        );
    }

    let degrees: Vec<usize> = match RECIPES
        .iter()
        .find(|(name, _, _)| name.eq_ignore_ascii_case(request.trim()))
    {
        Some((_, recipe_degrees, _)) => recipe_degrees.to_vec(),
        None => {
            let parsed: Result<Vec<usize>, _> = request
                .split(['-', ',', ' '])
                .filter(|s| !s.is_empty())
                .map(|s| s.trim().parse::<usize>())
                .collect();
            match parsed {
                Ok(list)
                    if !list.is_empty()
                        && list.iter().all(|&d| d >= 1 && d <= steps.len()) =>
                {
                    list
                }
                _ => {
                    return Err(format!(
                        "request must be a mood ({}) or degrees like '1-6-7-1' \
                         within 1-{}",
                        moods().join(", "),
                        steps.len()
                    ))
                }
            }
        }
    };

    let mut chords = Vec::with_capacity(degrees.len());
    for &degree in &degrees {
        let index = degree - 1;
        let quality = quality_of(steps, index);
        chords.push(PlannedChord {
            numeral: numeral(degree, quality),
            root_pc: (root_pc + steps[index % steps.len()]) % 12,
            quality,
        });
    }

    // Voice-leading check on root motion (power-chord world: roots and
    // fifths move in parallel, so root motion IS the voice leading).
    let mut notes = Vec::new();
    if let Some((_, _, description)) = RECIPES
        .iter()
        .find(|(name, _, _)| name.eq_ignore_ascii_case(request.trim()))
    {
        notes.push(format!("recipe: {description}"));
    }
    for pair in chords.windows(2) {
        let a = pair[0].root_pc as i16;
        let b = pair[1].root_pc as i16;
        let up = (b - a).rem_euclid(12);
        let motion = up.min(12 - up);
        let verdict = match motion {
            0 => "static (same root - fine as a pedal, dull twice in a row)",
            1 => "半 semitone slide - maximum menace, use sparingly",
            2 => "smooth step",
            5 => "fourth - the strongest functional pull",
            7 => "fifth - classic cadence motion",
            6 => "tritone - jarring on purpose; land it on an accent",
            _ => "leap - fine if the rhythm carries it",
        };
        notes.push(format!(
            "{} -> {}: {} ({} semitones)",
            pair[0].numeral, pair[1].numeral, verdict, motion
        ));
    }
    let distinct: std::collections::HashSet<u8> =
        chords.iter().map(|c| c.root_pc).collect();
    if distinct.len() == 1 {
        notes.push(
            "WARNING: single-root progression - this is a pedal, not harmony; \
             fine for a breakdown, weak for a chorus"
                .into(),
        );
    }
    Ok(HarmonyPlan { chords, notes })
}

pub fn describe(plan: &HarmonyPlan, scale_spec: &str, bars_per_chord: u32) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let mut out = format!("HARMONY PLAN in {scale_spec}:\n");
    for (i, chord) in plan.chords.iter().enumerate() {
        out.push_str(&format!(
            "  bar {:>2}+: {} = {}{}\n",
            i as u32 * bars_per_chord + 1,
            chord.numeral,
            names[chord.root_pc as usize],
            match chord.quality {
                "5" => "5",
                "m" => "m (or m5 as power chord)",
                "maj" => " (or 5 as power chord)",
                "dim" => "dim (use the b5 as color, power chord on the root)",
                _ => "",
            }
        ));
    }
    out.push_str("Voice leading:\n");
    for note in &plan.notes {
        out.push_str(&format!("  {note}\n"));
    }
    out.push_str(
        "Apply: write the roots as low power chords with replace_measures, or \
         seed generate_riff per chord section with the chord root as root_pc.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epic_recipe_in_a_minor() {
        let plan = plan("A natural minor", "epic").expect("plans");
        let numerals: Vec<&str> =
            plan.chords.iter().map(|c| c.numeral.as_str()).collect();
        assert_eq!(numerals, vec!["i", "VI", "VII", "i"]);
        // A minor: VI = F (pc 5), VII = G (pc 7).
        assert_eq!(plan.chords[1].root_pc, 5);
        assert_eq!(plan.chords[2].root_pc, 7);
        let text = describe(&plan, "A natural minor", 2);
        assert!(text.contains("F"), "{text}");
        assert!(text.contains("Voice leading"), "{text}");
    }

    #[test]
    fn explicit_degrees_and_pedal_warning() {
        let plan = plan("E phrygian dominant", "1-2-1-1").expect("plans");
        assert_eq!(plan.chords.len(), 4);
        // Phrygian dominant degree 2 is the b2 - one semitone up from E.
        assert_eq!(plan.chords[1].root_pc, (4 + 1) % 12);
        let single = super::plan("A natural minor", "1-1-1-1").expect("plans");
        assert!(single.notes.iter().any(|n| n.contains("pedal")));
    }

    #[test]
    fn rejects_nonsense() {
        assert!(plan("A natural minor", "yacht").is_err());
        assert!(plan("A hirajoshi", "1-2-3").is_err()); // 5-note scale
        assert!(plan("X major", "epic").is_err());
    }
}
