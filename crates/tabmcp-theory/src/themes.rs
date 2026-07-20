//! Theme tracker: musical memory across song sections. Finds each
//! section's motif, names the relations between them (restated, varied,
//! inverted, retrograde, fragment, extended), and detects call-and-response
//! phrasing inside sections. This is how the AI remembers Motif A from the
//! verse when it writes the outro.

use std::collections::HashMap;

use tabmcp_model::Measure;

use crate::critique;
use crate::fingering::Tuning;

pub struct SectionTheme {
    pub label: String,
    pub from_measure: u32,
    pub to_measure: u32,
    /// The section's dominant interval motif (empty = no recurring figure).
    pub motif: Vec<i8>,
}

pub struct ThemeRelation {
    pub from_index: usize,
    pub to_index: usize,
    pub relation: &'static str,
}

pub struct CallResponse {
    pub section: String,
    /// 1-based measure numbers of the paired phrases.
    pub call_measure: u32,
    pub response_measure: u32,
}

pub struct ThemeReport {
    pub sections: Vec<SectionTheme>,
    pub relations: Vec<ThemeRelation>,
    pub call_response: Vec<CallResponse>,
}

fn relate(a: &[i8], b: &[i8]) -> Option<&'static str> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    if a == b {
        return Some("restates");
    }
    let inverted: Vec<i8> = a.iter().map(|&i| -i).collect();
    if b == inverted.as_slice() {
        return Some("inverts");
    }
    let reversed: Vec<i8> = a.iter().rev().copied().collect();
    if b == reversed.as_slice() {
        return Some("retrogrades");
    }
    let retro_inverted: Vec<i8> = inverted.iter().rev().copied().collect();
    if b == retro_inverted.as_slice() {
        return Some("retrograde-inverts");
    }
    if b.len() >= 2 && b.len() < a.len() && a.windows(b.len()).any(|w| w == b) {
        return Some("fragments");
    }
    if a.len() >= 2 && a.len() < b.len() && b.windows(a.len()).any(|w| w == a) {
        return Some("extends");
    }
    if a.len() == b.len() {
        let same = a.iter().zip(b).filter(|(x, y)| x == y).count();
        if same * 10 >= a.len() * 6 {
            return Some("varies");
        }
    }
    None
}

/// Per-measure phrase signature for call-and-response detection.
struct Phrase {
    number: u32,
    rhythm: Vec<u64>,
    pitches: Vec<u8>,
    last_direction: i8,
}

fn phrases(measures: &[Measure], open: &HashMap<u32, u8>) -> Vec<Phrase> {
    let mut out = Vec::new();
    for measure in measures {
        let mut rhythm = Vec::new();
        let mut pitches = Vec::new();
        for beat in &measure.beats {
            for voice in &beat.voices {
                for note in &voice.notes {
                    if note.tied {
                        continue;
                    }
                    if let Some(&o) = open.get(&note.string) {
                        rhythm.push(beat.start_tick.saturating_sub(measure.start_tick));
                        pitches.push(o.saturating_add(note.fret as u8));
                    }
                }
            }
        }
        if pitches.len() < 2 {
            continue;
        }
        let last_direction =
            (pitches[pitches.len() - 1] as i16 - pitches[pitches.len() - 2] as i16).signum() as i8;
        out.push(Phrase {
            number: measure.number,
            rhythm,
            pitches,
            last_direction,
        });
    }
    out
}

/// Analyze themes across sections. Each entry is (label, from, to, measures).
pub fn analyze_themes(
    sections: &[(String, u32, u32, Vec<Measure>)],
    tuning: Tuning,
) -> ThemeReport {
    let open: HashMap<u32, u8> = tuning.iter().copied().collect();
    let section_themes: Vec<SectionTheme> = sections
        .iter()
        .map(|(label, from, to, measures)| {
            let report = critique::critique(measures, tuning);
            SectionTheme {
                label: label.clone(),
                from_measure: *from,
                to_measure: *to,
                motif: report.top_motif,
            }
        })
        .collect();

    let mut relations = Vec::new();
    for j in 1..section_themes.len() {
        // Relate each section to the EARLIEST section it echoes.
        for i in 0..j {
            if let Some(relation) = relate(&section_themes[i].motif, &section_themes[j].motif) {
                relations.push(ThemeRelation {
                    from_index: i,
                    to_index: j,
                    relation,
                });
                break;
            }
        }
    }

    let mut call_response = Vec::new();
    for (label, _, _, measures) in sections {
        let ps = phrases(measures, &open);
        for pair in ps.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if b.number == a.number + 1
                && a.rhythm == b.rhythm
                && a.pitches != b.pitches
                && a.last_direction != 0
                && b.last_direction != 0
                && a.last_direction != b.last_direction
            {
                call_response.push(CallResponse {
                    section: label.clone(),
                    call_measure: a.number,
                    response_measure: b.number,
                });
            }
        }
    }

    ThemeReport {
        sections: section_themes,
        relations,
        call_response,
    }
}

pub fn describe(report: &ThemeReport) -> String {
    let mut out = String::from("THEME MAP (musical memory):\n");
    let mut letters: Vec<Option<char>> = vec![None; report.sections.len()];
    let mut next_letter = b'A';
    for (i, section) in report.sections.iter().enumerate() {
        let echo = report.relations.iter().find(|r| r.to_index == i);
        let line = match echo {
            Some(r) => {
                letters[i] = letters[r.from_index];
                format!(
                    "  {} (m{}-{}): {} motif {} from \"{}\"\n",
                    section.label,
                    section.from_measure,
                    section.to_measure,
                    r.relation,
                    letters[r.from_index]
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".into()),
                    report.sections[r.from_index].label,
                )
            }
            None if !section.motif.is_empty() => {
                let letter = next_letter as char;
                letters[i] = Some(letter);
                next_letter = (next_letter + 1).min(b'Z');
                format!(
                    "  {} (m{}-{}): introduces motif {} {:?}\n",
                    section.label, section.from_measure, section.to_measure, letter, section.motif,
                )
            }
            None => format!(
                "  {} (m{}-{}): no recurring figure\n",
                section.label, section.from_measure, section.to_measure,
            ),
        };
        out.push_str(&line);
    }
    if report.call_response.is_empty() {
        out.push_str(
            "Call & response: none detected — paired phrases (same rhythm, answering \
             contour) make riffs conversational\n",
        );
    } else {
        for cr in &report.call_response {
            out.push_str(&format!(
                "Call & response: {} m{} asks, m{} answers (same rhythm, opposite contour)\n",
                cr.section, cr.call_measure, cr.response_measure,
            ));
        }
    }
    let introduced = letters.iter().flatten().collect::<std::collections::HashSet<_>>();
    if introduced.len() > 1 && report.relations.is_empty() {
        out.push_str(
            "NOTE: multiple motifs but zero cross-section relations — the song forgets \
             its own material; bring an earlier motif back (vary_riff it)\n",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tabmcp_model::{Beat, Duration, Note, NoteEffects, Tuplet, Voice};

    const STANDARD: &[(u32, u8)] = &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];

    fn measure(number: u32, steps: &[(u32, u32)]) -> Measure {
        let start = 960 * (1 + 4 * (number as u64 - 1));
        Measure {
            number,
            start_tick: start,
            key_signature: 0,
            beats: steps
                .iter()
                .enumerate()
                .map(|(j, &(string, fret))| Beat {
                    start_tick: start + j as u64 * 480,
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
    fn relations_cover_the_classical_devices() {
        assert_eq!(relate(&[3, 2, 2], &[3, 2, 2]), Some("restates"));
        assert_eq!(relate(&[3, 2, 2], &[-3, -2, -2]), Some("inverts"));
        assert_eq!(relate(&[3, 2, 1], &[1, 2, 3]), Some("retrogrades"));
        assert_eq!(relate(&[3, 2, 1], &[-1, -2, -3]), Some("retrograde-inverts"));
        assert_eq!(relate(&[3, 2, 2, 5], &[2, 2]), Some("fragments"));
        assert_eq!(relate(&[2, 2], &[3, 2, 2, 5]), Some("extends"));
        assert_eq!(relate(&[3, 2, 2], &[5, 7]), None);
    }

    #[test]
    fn theme_map_tracks_motifs_across_sections() {
        // Verse: figure with intervals repeated twice per section.
        let a1 = [(6u32, 0u32), (6, 3), (6, 5), (6, 7)];
        let verse = vec![measure(1, &a1), measure(2, &a1)];
        // Chorus: inverted figure (descending same steps), repeated.
        let inv = [(6u32, 12u32), (6, 9), (6, 7), (6, 5)];
        let chorus = vec![measure(3, &inv), measure(4, &inv)];
        let report = analyze_themes(
            &[
                ("Verse".into(), 1, 2, verse),
                ("Chorus".into(), 3, 4, chorus),
            ],
            STANDARD,
        );
        assert_eq!(report.sections.len(), 2);
        assert_eq!(report.relations.len(), 1, "{}", describe(&report));
        assert_eq!(report.relations[0].relation, "inverts");
        let text = describe(&report);
        assert!(text.contains("introduces motif A"), "{text}");
        assert!(text.contains("inverts motif A"), "{text}");
    }

    #[test]
    fn call_and_response_pairs_detected() {
        // Same rhythm, different pitches, opposite final direction.
        let call = [(6u32, 0u32), (6, 3), (6, 5), (6, 7)]; // ends rising
        let response = [(6u32, 7u32), (6, 5), (6, 3), (6, 0)]; // ends falling
        let section = vec![measure(1, &call), measure(2, &response)];
        let report = analyze_themes(&[("Verse".into(), 1, 2, section)], STANDARD);
        assert_eq!(report.call_response.len(), 1, "{}", describe(&report));
        assert_eq!(report.call_response[0].call_measure, 1);
        assert_eq!(report.call_response[0].response_measure, 2);
    }
}
