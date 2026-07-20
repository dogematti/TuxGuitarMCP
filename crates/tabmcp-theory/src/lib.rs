//! Music-theory core. Milestone 1 only needs pitch naming; scale/key/chord
//! analysis lands here in Phase 3+.

pub mod analysis;
pub mod arrangement;
pub mod cells;
pub mod critique;
pub mod difficulty;
pub mod emotion;
pub mod fingering;
pub mod generation;
pub mod pitch;
pub mod realism;
pub mod search;
pub mod styles;
pub mod themes;
pub mod transforms;

pub use analysis::{
    detect_scales, explain, melodic_intervals, tonal_center, transpose_measures, NoteEvent,
    ScaleCandidate, TransposeProblem,
};
pub use pitch::{note_name, parse_note, pitch_class_name, tuning_preset, TUNING_PRESETS};
