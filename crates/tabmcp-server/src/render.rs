//! Rendering helpers shared by the audio-ear tools: locating fluidsynth and
//! a soundfont, rendering MIDI to WAV, and splitting a multitrack MIDI into
//! per-track stems (tempo/meta track preserved).

use std::path::{Path, PathBuf};

/// Locate a usable fluidsynth binary.
pub fn find_fluidsynth() -> Result<String, String> {
    [
        "/opt/homebrew/bin/fluidsynth",
        "/usr/local/bin/fluidsynth",
        "fluidsynth",
    ]
    .iter()
    .find(|c| {
        std::process::Command::new(c)
            .arg("--version")
            .output()
            .is_ok()
    })
    .map(|c| c.to_string())
    .ok_or_else(|| "fluidsynth not found — install it with: brew install fluid-synth".into())
}

/// Locate a soundfont: user override first, then the one TuxGuitar ships.
pub fn find_soundfont() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let override_font = PathBuf::from(&home).join(".tuxguitar-mcp/soundfont.sf2");
    if override_font.exists() {
        return Ok(override_font);
    }
    if let Ok(apps) = std::fs::read_dir("/Applications") {
        for app in apps.flatten() {
            if app
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .contains("tuxguitar")
            {
                let candidate = app
                    .path()
                    .join("Contents/MacOS/share/soundfont/MagicSFver2.sf2");
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
    }
    Err("no soundfont found — place a GM .sf2 at ~/.tuxguitar-mcp/soundfont.sf2".into())
}

/// Render a MIDI file to WAV via fluidsynth.
pub fn render_wav(midi: &Path, wav: &Path) -> Result<(), String> {
    let fluidsynth = find_fluidsynth()?;
    let soundfont = find_soundfont()?;
    let output = std::process::Command::new(fluidsynth)
        .args(["-ni", "-r", "44100", "-F"])
        .arg(wav)
        .arg(&soundfont)
        .arg(midi)
        .output()
        .map_err(|e| format!("fluidsynth failed to start: {e}"))?;
    if !output.status.success() || !wav.exists() {
        return Err(format!(
            "fluidsynth render failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    Ok(())
}

/// Split a format-1 MIDI into per-content-track files. Returns
/// (stem_index_within_content_tracks, path) pairs. Tracks without any
/// NoteOn are treated as meta/tempo tracks and included in EVERY stem.
pub fn split_stems(midi_path: &Path, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let bytes = std::fs::read(midi_path).map_err(|e| format!("cannot read MIDI: {e}"))?;
    let smf = midly::Smf::parse(&bytes).map_err(|e| format!("cannot parse MIDI: {e}"))?;

    let has_notes = |track: &midly::Track| {
        track.iter().any(|event| {
            matches!(
                event.kind,
                midly::TrackEventKind::Midi {
                    message: midly::MidiMessage::NoteOn { .. },
                    ..
                }
            )
        })
    };
    let meta_tracks: Vec<usize> = smf
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| !has_notes(t))
        .map(|(i, _)| i)
        .collect();
    let content_tracks: Vec<usize> = smf
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| has_notes(t))
        .map(|(i, _)| i)
        .collect();

    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut stems = Vec::new();
    for (stem_number, &content) in content_tracks.iter().enumerate() {
        let mut stem = midly::Smf::new(smf.header);
        for &meta in &meta_tracks {
            stem.tracks.push(smf.tracks[meta].clone());
        }
        stem.tracks.push(smf.tracks[content].clone());
        let path = out_dir.join(format!("stem-{}.mid", stem_number + 1));
        let mut buffer = Vec::new();
        stem.write(&mut buffer)
            .map_err(|e| format!("cannot write stem: {e}"))?;
        std::fs::write(&path, buffer).map_err(|e| e.to_string())?;
        stems.push(path);
    }
    Ok(stems)
}
