//! Style guide: composition recipes an AI client folds into its writing.
//! Every field references EXISTING machinery — scales from the catalog,
//! drum template names, effect objects, rhythmic cells as note patterns.

pub struct StyleGuide {
    pub name: &'static str,
    pub scales: &'static str,
    pub tempo: &'static str,
    pub cells: &'static str,
    pub techniques: &'static str,
    pub drums: &'static str,
    pub devices: &'static str,
}

pub const STYLES: &[StyleGuide] = &[
    StyleGuide { name: "thrash", scales: "natural minor, harmonic minor, chromatic passing tones", tempo: "160-220", cells: "straight 16th chug, gallop (8th+two 16ths)", techniques: "palmMute on chugs, fast alternate picking", drums: "d-beat verses, blast fills", devices: "E5->F5 semitone power-chord stabs; tritone riffs; last-bar chromatic climb" },
    StyleGuide { name: "death metal", scales: "phrygian, locrian, half-whole diminished", tempo: "180-260", cells: "blast 16ths, tremolo-picked 16ths", techniques: "tremoloPicking flag, pinch harmonics, palmMute", drums: "blast", devices: "chromatic descent riffs; tritone pedal; abrupt halftime drops" },
    StyleGuide { name: "black metal", scales: "natural minor, phrygian", tempo: "180-240", cells: "straight tremolo 8ths/16ths on chord tones", techniques: "tremoloPicking over minor-chord arpeggios, letRing", drums: "blast", devices: "minor triads tremolo-picked as melody; long single-chord stretches" },
    StyleGuide { name: "doom", scales: "minor pentatonic, phrygian", tempo: "55-90", cells: "half/whole notes, dragging triplets", techniques: "full-tone bends, slides, vibrato, letRing", drums: "halftime", devices: "Iommi tritone bend (fret 0->1 area); SPACE between hits; unison low-string doubles" },
    StyleGuide { name: "groove metal", scales: "phrygian dominant, blues", tempo: "90-130", cells: "syncopated 16ths, 2+2+3 groupings", techniques: "palmMute, pinch squeals on accents, dead notes", drums: "halftime or rock", devices: "one riff rhythmically displaced by an 8th; b2 stabs against open-string pedal" },
    StyleGuide { name: "djent", scales: "phrygian dominant, altered, chromatic", tempo: "110-150", cells: "7/8 and 5/4 bars (set_time_signature), 2+2+3+2, implied polymeter (3-note cell over 4/4)", techniques: "low-string chug + clean wide-interval melody in voice 1", drums: "metal-gallop verses, halftime breakdowns via target_track", devices: "same riff re-barred across meters; octave-jump accents on string 1-2" },
    StyleGuide { name: "metalcore", scales: "natural minor, power chords", tempo: "120-160", cells: "halftime 8th-note breakdown chug", techniques: "open-string chug, dissonant octave stabs (interval 13)", drums: "halftime breakdown, d-beat verses", devices: "breakdown = rhythm only, one pitch; guitar/kick unison" },
    StyleGuide { name: "power metal", scales: "major, harmonic minor", tempo: "160-200", cells: "straight gallop throughout", techniques: "fast scale runs, harmonized leads", drums: "metal-gallop", devices: "generate_harmony in 3rds over the lead = instant dual guitars; V-i harmonic-minor cadences" },
    StyleGuide { name: "classic heavy", scales: "minor pentatonic, dorian", tempo: "120-160", cells: "8th-note drive, gallop bridges", techniques: "double stops, unison bends, vibrato", drums: "rock", devices: "riff = pentatonic box + open low string; twin-lead harmonies" },
    StyleGuide { name: "punk", scales: "major/minor pentatonic", tempo: "160-210", cells: "straight downpicked 8ths", techniques: "power chords only, no ornament", drums: "punk or d-beat", devices: "3-chord I-IV-V turnarounds; whole-bar chord pushes" },
    StyleGuide { name: "blues rock", scales: "blues, mixolydian, major blues", tempo: "80-140", cells: "shuffle = triplet pairs (tuplet 3:2 with middle rest), 12-bar form", techniques: "bends to the b3/4, wide vibrato, slides", drums: "rock", devices: "call-and-response phrasing; turnaround lick bar 12; copy_measures for the 12-bar form" },
    StyleGuide { name: "funk rock", scales: "dorian, minor pentatonic", tempo: "95-115", cells: "16th grid with ghost notes on the e/a", techniques: "ghostNote + deadNote + staccato, single-note riffs", drums: "rock with ghosted snare", devices: "chromatic approach into the root; one-bar riff, displacement variations" },
    StyleGuide { name: "jazz fusion", scales: "melodic minor modes (lydian dominant, altered), whole tone", tempo: "100-180", cells: "swung 8ths (triplet pairs), 5- and 7-note groupings", techniques: "legato (hammer flags), wide intervals", drums: "rock (ride-focused)", devices: "superimpose lydian dominant over dominant chords; ii-V motion in the bass" },
    StyleGuide { name: "flamenco metal", scales: "phrygian dominant, double harmonic", tempo: "100-140", cells: "triplet 16th bursts (rasgueado feel)", techniques: "fast triplet picking, staccato chords", drums: "halftime", devices: "Andalusian cadence Am-G-F-E as power chords; b2 trills" },
    StyleGuide { name: "surf", scales: "hirajoshi, harmonic minor", tempo: "140-180", cells: "straight tremolo 8ths/16ths", techniques: "tremoloPicking melody on strings 1-2", drums: "rock", devices: "minor-key double-picked melody; dramatic whole-band stops" },
];

pub fn style_guide(name: &str) -> Option<&'static StyleGuide> {
    let wanted = name.trim().to_ascii_lowercase();
    STYLES.iter().find(|s| s.name == wanted)
}

pub fn describe(guide: &StyleGuide) -> String {
    format!(
        "STYLE: {}\nScales: {}\nTempo: {} BPM\nRhythmic cells: {}\nTechniques: {}\nDrum styles: {}\nSignature devices: {}\n",
        guide.name, guide.scales, guide.tempo, guide.cells, guide.techniques, guide.drums, guide.devices
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
        assert_eq!(STYLES.len(), 15);
        let text = describe(style_guide("doom").unwrap());
        assert!(text.contains("Iommi"), "{text}");
    }
}
