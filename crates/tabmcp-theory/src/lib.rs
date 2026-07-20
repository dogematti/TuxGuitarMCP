//! Music-theory core. Milestone 1 only needs pitch naming; scale/key/chord
//! analysis lands here in Phase 3+.

pub mod pitch;

pub use pitch::{note_name, pitch_class_name};
