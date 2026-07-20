//! Wire types for measure-level content (`read_measures`) and the editor
//! selection (`read_selection`). Same conventions as `lib.rs`: camelCase,
//! unknown fields tolerated. These derive `JsonSchema` because they are also
//! returned as structured content by MCP tools.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn is_false(b: &bool) -> bool {
    !*b
}

/// Tuplet division: `enters` notes in the time of `times` (3:2 = triplet).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tuplet {
    pub enters: u32,
    pub times: u32,
}

impl Tuplet {
    pub fn is_normal(&self) -> bool {
        self.enters == 1 && self.times == 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Duration {
    /// Note value: 1 = whole, 2 = half, 4 = quarter, ... 64 = sixty-fourth.
    pub value: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dotted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub double_dotted: bool,
    pub tuplet: Tuplet,
}

/// A harmonic effect. `kind` serializes as `type` on the wire:
/// "natural" (N.H), "artificial" (A.H), "tapped" (T.H), "pinch" (P.H),
/// or "semi" (S.H). `data` is the octave offset for artificial/tapped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HarmonicEffect {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<i32>,
}

impl HarmonicEffect {
    pub fn pinch() -> Self {
        HarmonicEffect {
            kind: "pinch".into(),
            data: None,
        }
    }
    pub fn natural() -> Self {
        HarmonicEffect {
            kind: "natural".into(),
            data: None,
        }
    }
}

/// One point of a bend curve: `position` 0..=12 spans the note's duration,
/// `value` is the bend height in semitones (2 = full tone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BendPoint {
    pub position: u32,
    pub value: u32,
}

/// A bend. An empty `points` list means "standard full-tone bend"
/// (the bridge writes 0->2 semitones over the first half of the note).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BendEffect {
    #[serde(default)]
    pub points: Vec<BendPoint>,
}

/// Accept both the legacy boolean form (`"harmonic": true`) and the
/// parameterized object form on the wire.
mod effect_compat {
    use super::{BendEffect, GraceEffect, HarmonicEffect, TremoloPickingEffect, TrillEffect};
    use serde::{Deserialize, Deserializer};

    macro_rules! bool_or_full {
        ($name:ident, $ty:ty) => {
            pub fn $name<'de, D: Deserializer<'de>>(d: D) -> Result<Option<$ty>, D::Error> {
                #[derive(Deserialize)]
                #[serde(untagged)]
                enum Raw {
                    Legacy(bool),
                    Full($ty),
                }
                Ok(Option::<Raw>::deserialize(d)?.and_then(|raw| match raw {
                    Raw::Legacy(true) => Some(<$ty>::default()),
                    Raw::Legacy(false) => None,
                    Raw::Full(v) => Some(v),
                }))
            }
        };
    }

    bool_or_full!(grace, GraceEffect);
    bool_or_full!(trill, TrillEffect);
    bool_or_full!(tremolo_picking, TremoloPickingEffect);

    pub fn harmonic<'de, D: Deserializer<'de>>(d: D) -> Result<Option<HarmonicEffect>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Legacy(bool),
            Full(HarmonicEffect),
        }
        Ok(Option::<Raw>::deserialize(d)?.and_then(|raw| match raw {
            Raw::Legacy(true) => Some(HarmonicEffect::natural()),
            Raw::Legacy(false) => None,
            Raw::Full(h) => Some(h),
        }))
    }

    pub fn bend<'de, D: Deserializer<'de>>(d: D) -> Result<Option<BendEffect>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Legacy(bool),
            Full(BendEffect),
        }
        Ok(Option::<Raw>::deserialize(d)?.and_then(|raw| match raw {
            Raw::Legacy(true) => Some(BendEffect::default()),
            Raw::Legacy(false) => None,
            Raw::Full(b) => Some(b),
        }))
    }
}

/// Per-note effect flags. Only flags that are set travel on the wire.
/// Harmonics and bends carry parameters; the remaining complex effects
/// (grace, trill, tremolo picking/bar) are presence flags until a later
/// protocol version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteEffects {
    #[serde(default, skip_serializing_if = "is_false")]
    pub vibrato: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dead_note: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub slide: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hammer: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ghost_note: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub accent: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub heavy_accent: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub palm_mute: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub staccato: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub let_ring: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tapping: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub slapping: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub popping: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub fade_in: bool,
    #[serde(
        default,
        deserialize_with = "effect_compat::bend",
        skip_serializing_if = "Option::is_none"
    )]
    pub bend: Option<BendEffect>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tremolo_bar: bool,
    #[serde(
        default,
        deserialize_with = "effect_compat::harmonic",
        skip_serializing_if = "Option::is_none"
    )]
    pub harmonic: Option<HarmonicEffect>,
    #[serde(
        default,
        deserialize_with = "effect_compat::grace",
        skip_serializing_if = "Option::is_none"
    )]
    pub grace: Option<GraceEffect>,
    #[serde(
        default,
        deserialize_with = "effect_compat::trill",
        skip_serializing_if = "Option::is_none"
    )]
    pub trill: Option<TrillEffect>,
    #[serde(
        default,
        deserialize_with = "effect_compat::tremolo_picking",
        skip_serializing_if = "Option::is_none"
    )]
    pub tremolo_picking: Option<TremoloPickingEffect>,
}

/// A trill: rapid alternation with a second fret. `fret` 0 means "auto"
/// (the bridge picks a whole tone above the note); `speed` is the
/// alternation subdivision (8, 16 or 32; default 32).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrillEffect {
    #[serde(default)]
    pub fret: u32,
    #[serde(default = "default_trill_speed")]
    pub speed: u32,
}

impl Default for TrillEffect {
    fn default() -> Self {
        Self { fret: 0, speed: 32 }
    }
}

fn default_trill_speed() -> u32 {
    32
}

/// Tremolo picking: the note is repicked at `speed` (8, 16 or 32 =
/// eighths, sixteenths, thirty-seconds; default 16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TremoloPickingEffect {
    #[serde(default = "default_tremolo_speed")]
    pub speed: u32,
}

impl Default for TremoloPickingEffect {
    fn default() -> Self {
        Self { speed: 16 }
    }
}

fn default_tremolo_speed() -> u32 {
    16
}

/// A grace note before (or on) the beat. `fret` absent means "auto" (two
/// frets below the note). `duration`: 1 = 64th, 2 = 32nd (default),
/// 3 = 16th. `transition`: "none", "slide", "bend" or "hammer" (default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraceEffect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fret: Option<u32>,
    #[serde(default = "default_grace_duration")]
    pub duration: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub on_beat: bool,
    #[serde(default = "default_grace_transition")]
    pub transition: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dead: bool,
}

impl Default for GraceEffect {
    fn default() -> Self {
        Self {
            fret: None,
            duration: 2,
            on_beat: false,
            transition: "hammer".into(),
            dead: false,
        }
    }
}

fn default_grace_duration() -> u32 {
    2
}

fn default_grace_transition() -> String {
    "hammer".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// String number, 1-based, 1 = highest-sounding string.
    pub string: u32,
    pub fret: u32,
    pub velocity: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tied: bool,
    #[serde(default)]
    pub effects: NoteEffects,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    /// Voice index inside the beat (TuxGuitar has 2 voices per beat).
    pub index: u32,
    pub duration: Duration,
    /// True when this voice is a rest (no notes).
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_rest: bool,
    #[serde(default)]
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Beat {
    pub start_tick: u64,
    pub voices: Vec<Voice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Measure {
    /// 1-based measure number.
    pub number: u32,
    /// Tick at which this measure starts. Beat `startTick`s are absolute;
    /// when applying edits, beats are positioned by their offset from this
    /// value, so content can be written with `startTick: 0` + offsets too.
    #[serde(default)]
    pub start_tick: u64,
    /// TuxGuitar key signature code (0 = C major / A minor).
    #[serde(default)]
    pub key_signature: i32,
    pub beats: Vec<Beat>,
}

/// Result of `read_measures`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRange {
    pub track_number: u32,
    pub from_measure: u32,
    pub to_measure: u32,
    pub measures: Vec<Measure>,
    pub revision: u64,
    pub document_id: String,
}

/// Result of `apply_changeset`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub new_revision: u64,
    pub measures_replaced: u32,
    /// Measures appended to the song because the range extended past its end.
    pub measures_added: u32,
    pub notes_before: u32,
    pub notes_after: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveCopyResult {
    /// True when TuxGuitar's Save-As dialog was opened for the user.
    pub dialog_opened: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CaretPosition {
    pub track_number: u32,
    pub measure_number: u32,
    pub tick: u64,
    pub string_number: u32,
}

/// Result of `read_selection`. `active` is false when nothing is selected;
/// the caret is reported either way.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_measure: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_measure: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caret: Option<CaretPosition>,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harmonic_and_bend_round_trip() {
        let effects = NoteEffects {
            palm_mute: true,
            harmonic: Some(HarmonicEffect::pinch()),
            bend: Some(BendEffect {
                points: vec![
                    BendPoint {
                        position: 0,
                        value: 0,
                    },
                    BendPoint {
                        position: 6,
                        value: 2,
                    },
                ],
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&effects).unwrap();
        assert!(json.contains(r#""harmonic":{"type":"pinch"}"#), "{json}");
        assert!(
            json.contains(
                r#""bend":{"points":[{"position":0,"value":0},{"position":6,"value":2}]}"#
            ),
            "{json}"
        );
        let back: NoteEffects = serde_json::from_str(&json).unwrap();
        assert_eq!(back, effects);
    }

    #[test]
    fn legacy_boolean_effects_still_parse() {
        let json = r#"{"harmonic":true,"bend":true,"palmMute":true}"#;
        let effects: NoteEffects = serde_json::from_str(json).unwrap();
        assert_eq!(effects.harmonic, Some(HarmonicEffect::natural()));
        assert_eq!(effects.bend, Some(BendEffect::default()));
        let none: NoteEffects = serde_json::from_str(r#"{"harmonic":false}"#).unwrap();
        assert_eq!(none.harmonic, None);
    }
}
