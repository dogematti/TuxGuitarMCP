//! Style guide: composition recipes an AI client folds into its writing.
//! Every field references EXISTING machinery — scales from the catalog,
//! tunings from TUNING_PRESETS, drum template names, effect objects,
//! rhythmic cells as note patterns. Every style fills the SAME rubric so
//! evaluation stays consistent across genres.

pub struct StyleGuide {
    pub name: &'static str,
    pub scales: &'static str,
    pub tempo: &'static str,
    pub cells: &'static str,
    pub techniques: &'static str,
    pub drums: &'static str,
    pub devices: &'static str,
    /// Suggested tuning presets (names accepted by tuxguitar_create_track).
    pub tuning: &'static str,
    /// Typical meters (set via tuxguitar_set_time_signature).
    pub meters: &'static str,
    /// Song-section arc: what the structure usually looks like.
    pub sections: &'static str,
    /// The mood the style aims for.
    pub mood: &'static str,
    /// Technical difficulty for a human player.
    pub difficulty: &'static str,
    /// Things that break the style — do NOT write these.
    pub avoid: &'static str,
    /// Numeric BPM range for evaluation (matches `tempo`).
    pub tempo_range: (u16, u16),
    /// Target syncopation window 0..1 for the AI Ear style check — below it
    /// the style reads metronomic, above it unmoored.
    pub syncopation: (f64, f64),
}

pub const STYLES: &[StyleGuide] = &[
    StyleGuide { name: "thrash", scales: "natural minor, harmonic minor, chromatic passing tones", tempo: "160-220", cells: "straight 16th chug, gallop (8th+two 16ths)", techniques: "palmMute on chugs, fast alternate picking", drums: "d-beat verses, blast fills", devices: "E5->F5 semitone power-chord stabs; tritone riffs; last-bar chromatic climb", tuning: "6-string standard or 6-string E-flat", meters: "4/4", sections: "riff intro -> verse chug -> pre-chorus stabs -> chorus -> solo -> outro climb", mood: "aggressive, urgent", difficulty: "advanced (fast alternate picking)", avoid: "swing feel, major-key brightness, sparse arrangements", tempo_range: (160, 220), syncopation: (0.10, 0.50) },
    StyleGuide { name: "death metal", scales: "phrygian, locrian, half-whole diminished", tempo: "180-260", cells: "blast 16ths, tremolo-picked 16ths", techniques: "tremoloPicking flag, pinch harmonics, palmMute", drums: "blast", devices: "chromatic descent riffs; tritone pedal; abrupt halftime drops", tuning: "6-string drop C or 7-string B standard", meters: "4/4 with abrupt cuts", sections: "blast verse -> tremolo bridge -> halftime drop -> optional solo", mood: "brutal, relentless", difficulty: "advanced", avoid: "clean dynamics, pentatonic rock licks, pop song structure", tempo_range: (180, 260), syncopation: (0.10, 0.50) },
    StyleGuide { name: "black metal", scales: "natural minor, phrygian", tempo: "180-240", cells: "straight tremolo 8ths/16ths on chord tones", techniques: "tremoloPicking over minor-chord arpeggios, letRing", drums: "blast", devices: "minor triads tremolo-picked as melody; long single-chord stretches", tuning: "6-string standard", meters: "4/4", sections: "long-form riff cycles in 8-bar waves, minimal contrast, no stops", mood: "cold, hypnotic, epic", difficulty: "intermediate (endurance over precision)", avoid: "groove syncopation, blues bends, tight stop-start rhythm", tempo_range: (180, 240), syncopation: (0.0, 0.30) },
    StyleGuide { name: "doom", scales: "minor pentatonic, phrygian", tempo: "55-90", cells: "half/whole notes, dragging triplets", techniques: "full-tone bends, slides, vibrato, letRing", drums: "halftime", devices: "Iommi tritone bend (fret 0->1 area); SPACE between hits; unison low-string doubles", tuning: "6-string drop C or 6-string E-flat", meters: "4/4, slow 6/8", sections: "riff A stated long -> riff B longer -> late solo; tempo never moves", mood: "heavy, mournful, massive", difficulty: "easy notes, advanced feel", avoid: "fast runs, dense note counts, thin high voicings", tempo_range: (55, 90), syncopation: (0.10, 0.50) },
    StyleGuide { name: "groove metal", scales: "phrygian dominant, blues", tempo: "90-130", cells: "syncopated 16ths, 2+2+3 groupings", techniques: "palmMute, pinch squeals on accents, dead notes", drums: "halftime or rock", devices: "one riff rhythmically displaced by an 8th; b2 stabs against open-string pedal", tuning: "6-string drop D or 6-string drop C", meters: "4/4 with displaced accents", sections: "intro groove -> verse riff -> displaced variant (vary_riff!) -> breakdown", mood: "swaggering, menacing", difficulty: "intermediate", avoid: "straight metronomic 8ths, melodic excess", tempo_range: (90, 130), syncopation: (0.35, 0.70) },
    StyleGuide { name: "djent", scales: "phrygian dominant, altered, chromatic", tempo: "110-150", cells: "7/8 and 5/4 bars (set_time_signature), 2+2+3+2, implied polymeter (3-note cell over 4/4)", techniques: "low-string chug + clean wide-interval melody in voice 1", drums: "metal-gallop verses, halftime breakdowns via target_track", devices: "same riff re-barred across meters (vary_riff regroup!); octave-jump accents on string 1-2", tuning: "8-string F# standard or 7-string A standard", meters: "7/8, 5/4, polymetric 4/4", sections: "clean ambient intro -> regrouped chug verse -> open chorus -> breakdown", mood: "mechanical yet atmospheric", difficulty: "expert (rhythmic precision)", avoid: "shuffle feel, 12-bar forms, constant blast beats", tempo_range: (110, 150), syncopation: (0.35, 0.75) },
    StyleGuide { name: "metalcore", scales: "natural minor, power chords", tempo: "120-160", cells: "halftime 8th-note breakdown chug", techniques: "open-string chug, dissonant octave stabs (interval 13)", drums: "halftime breakdown, d-beat verses", devices: "breakdown = rhythm only, one pitch; guitar/kick unison", tuning: "6-string drop C or 7-string A standard", meters: "4/4, halftime breakdowns", sections: "intro -> d-beat verse -> chorus hook -> BREAKDOWN centerpiece -> outro", mood: "cathartic, heavy-vs-melodic", difficulty: "intermediate", avoid: "solos over the breakdown, jazz chords, swing", tempo_range: (120, 160), syncopation: (0.20, 0.60) },
    StyleGuide { name: "deathcore", scales: "phrygian, locrian, chromatic", tempo: "100-250", cells: "blast verses vs. sub-90 halftime breakdowns; tempo drops via set_tempo at the breakdown", techniques: "tremoloPicking verses, open-string chug, pinch squeals on breakdown accents, deadNote thuds", drums: "blast verses, halftime breakdowns via target_track", devices: "the DROP: cut everything to halftime + one low pitch; pedal-tone fills (vary_riff pedal); dissonant b2/tritone stabs over open-string chug", tuning: "7-string A standard or 8-string F# standard", meters: "4/4, halftime drops", sections: "blast verse -> tempo DROP breakdown -> second, heavier drop", mood: "menacing, punishing", difficulty: "advanced", avoid: "major tonality, groove swing, thin single-note verses", tempo_range: (100, 250), syncopation: (0.15, 0.60) },
    StyleGuide { name: "power metal", scales: "major, harmonic minor", tempo: "160-200", cells: "straight gallop throughout", techniques: "fast scale runs, harmonized leads", drums: "metal-gallop", devices: "generate_harmony in 3rds over the lead = instant dual guitars; V-i harmonic-minor cadences", tuning: "6-string standard", meters: "4/4, occasional 3/4", sections: "intro fanfare -> gallop verse -> pre-chorus -> anthemic chorus -> twin-lead solo", mood: "triumphant, soaring", difficulty: "advanced (speed + endurance)", avoid: "dissonance, downtuned chug, halftime breakdowns", tempo_range: (160, 200), syncopation: (0.15, 0.50) },
    StyleGuide { name: "classic heavy", scales: "minor pentatonic, dorian", tempo: "120-160", cells: "8th-note drive, gallop bridges", techniques: "double stops, unison bends, vibrato", drums: "rock", devices: "riff = pentatonic box + open low string; twin-lead harmonies", tuning: "6-string standard", meters: "4/4", sections: "riff intro -> verse -> chorus -> twin-lead bridge -> solo", mood: "confident, driving", difficulty: "intermediate", avoid: "extended-range chug, blast beats", tempo_range: (120, 160), syncopation: (0.15, 0.50) },
    StyleGuide { name: "punk", scales: "major/minor pentatonic", tempo: "160-210", cells: "straight downpicked 8ths", techniques: "power chords only, no ornament", drums: "punk or d-beat", devices: "3-chord I-IV-V turnarounds; whole-bar chord pushes", tuning: "6-string standard", meters: "4/4", sections: "count-in -> verse -> chorus -> verse -> chorus -> done (keep it SHORT)", mood: "raw, energetic", difficulty: "beginner", avoid: "solos, ornaments, more than four chords", tempo_range: (160, 210), syncopation: (0.0, 0.35) },
    StyleGuide { name: "blues rock", scales: "blues, mixolydian, major blues", tempo: "80-140", cells: "shuffle = triplet pairs (tuplet 3:2 with middle rest), 12-bar form", techniques: "bends to the b3/4, wide vibrato, slides", drums: "rock", devices: "call-and-response phrasing; turnaround lick bar 12; copy_measures for the 12-bar form", tuning: "6-string standard", meters: "4/4 shuffle, 12/8", sections: "12-bar form: head -> solo choruses -> head out", mood: "loose, expressive", difficulty: "intermediate (feel-critical)", avoid: "rigid grid quantization, metal techniques, dense chord stacks", tempo_range: (80, 140), syncopation: (0.20, 0.60) },
    StyleGuide { name: "funk rock", scales: "dorian, minor pentatonic", tempo: "95-115", cells: "16th grid with ghost notes on the e/a", techniques: "ghostNote + deadNote + staccato, single-note riffs", drums: "rock with ghosted snare", devices: "chromatic approach into the root; one-bar riff, displacement variations", tuning: "6-string standard", meters: "4/4 on a 16th grid", sections: "one-bar riff + variations -> bridge in a new key center -> riff out", mood: "playful, tight", difficulty: "advanced (ghost-note control)", avoid: "legato walls, power-chord walls, straight 8ths", tempo_range: (95, 115), syncopation: (0.45, 0.80) },
    StyleGuide { name: "jazz fusion", scales: "melodic minor modes (lydian dominant, altered), whole tone", tempo: "100-180", cells: "swung 8ths (triplet pairs), 5- and 7-note groupings", techniques: "legato (hammer flags), wide intervals", drums: "rock (ride-focused)", devices: "superimpose lydian dominant over dominant chords; ii-V motion in the bass", tuning: "6-string standard", meters: "4/4 swung, 7/4, 5/4", sections: "head -> solos over the form -> head; ii-V links between sections", mood: "sophisticated, exploratory", difficulty: "expert", avoid: "power chords, blast beats, a single pedal chord throughout", tempo_range: (100, 180), syncopation: (0.40, 0.80) },
    StyleGuide { name: "flamenco metal", scales: "phrygian dominant, double harmonic", tempo: "100-140", cells: "triplet 16th bursts (rasgueado feel)", techniques: "fast triplet picking, staccato chords", drums: "halftime", devices: "Andalusian cadence Am-G-F-E as power chords; b2 trills", tuning: "6-string standard or 6-string E-flat", meters: "4/4 with 3/4 phrases", sections: "solo-guitar intro -> Andalusian cadence riff -> triplet-burst bridge -> climax", mood: "fiery, dramatic", difficulty: "advanced (triplet bursts)", avoid: "blue notes, swing, sparse lazy rhythm", tempo_range: (100, 140), syncopation: (0.25, 0.65) },
    StyleGuide { name: "surf", scales: "hirajoshi, harmonic minor", tempo: "140-180", cells: "straight tremolo 8ths/16ths", techniques: "tremoloPicking melody on strings 1-2", drums: "rock", devices: "minor-key double-picked melody; dramatic whole-band stops", tuning: "6-string standard", meters: "4/4", sections: "melody A -> melody B -> full-band stops -> A out; keep it short", mood: "urgent retro cool", difficulty: "intermediate (tremolo stamina)", avoid: "palm-muted chug, downtuning, breakdowns", tempo_range: (140, 180), syncopation: (0.10, 0.40) },
];

pub fn style_guide(name: &str) -> Option<&'static StyleGuide> {
    let wanted = name.trim().to_ascii_lowercase();
    STYLES.iter().find(|s| s.name == wanted)
}

pub fn describe(guide: &StyleGuide) -> String {
    format!(
        "STYLE: {}\nMood: {}\nTuning: {}\nTempo: {} BPM\nMeters: {}\nScales: {}\nRhythmic cells: {}\nTechniques: {}\nDrum styles: {}\nSignature devices: {}\nSong sections: {}\nDifficulty: {}\nAVOID: {}\nEvaluation targets: tempo {}-{} BPM, syncopation {:.0}-{:.0}% (pass style=\"{}\" to tuxguitar_evaluate to check these)\n",
        guide.name,
        guide.mood,
        guide.tuning,
        guide.tempo,
        guide.meters,
        guide.scales,
        guide.cells,
        guide.techniques,
        guide.drums,
        guide.devices,
        guide.sections,
        guide.difficulty,
        guide.avoid,
        guide.tempo_range.0,
        guide.tempo_range.1,
        guide.syncopation.0 * 100.0,
        guide.syncopation.1 * 100.0,
        guide.name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lookup_and_listing() {
        assert!(style_guide("djent").is_some());
        assert!(style_guide("DJENT ").is_some());
        assert!(style_guide("yacht rock").is_none());
        assert_eq!(STYLES.len(), 16);
        let text = describe(style_guide("doom").unwrap());
        assert!(text.contains("Iommi"), "{text}");
        assert!(text.contains("Evaluation targets"), "{text}");
        assert!(text.contains("AVOID"), "{text}");
    }

    #[test]
    fn deathcore_recipe_has_the_drop() {
        let text = describe(style_guide("deathcore").unwrap());
        assert!(text.contains("DROP"), "{text}");
        assert!(text.contains("7-string A standard"), "{text}");
    }

    #[test]
    fn rubric_is_complete_for_every_style() {
        for style in STYLES {
            assert!(style.tempo_range.0 < style.tempo_range.1, "{}", style.name);
            assert!(
                style.syncopation.0 < style.syncopation.1 && style.syncopation.1 <= 1.0,
                "{}",
                style.name
            );
            for (field, value) in [
                ("tuning", style.tuning),
                ("meters", style.meters),
                ("sections", style.sections),
                ("mood", style.mood),
                ("difficulty", style.difficulty),
                ("avoid", style.avoid),
            ] {
                assert!(!value.is_empty(), "{}: empty {}", style.name, field);
            }
        }
    }
}
