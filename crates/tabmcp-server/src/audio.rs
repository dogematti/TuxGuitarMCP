//! Virtual ear v2: DSP analysis of a rendered mix.
//!
//! The score tells us what SHOULD sound; this module measures what actually
//! comes out of the synth: levels, clipping, quiet holes, and spectral
//! balance (low-end mud / harshness / dullness). Findings are rendered as
//! producer's notes the AI client can act on.

use std::path::Path;

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

#[derive(Debug, Clone)]
pub struct AudioReport {
    pub duration_seconds: f64,
    pub peak_dbfs: f64,
    pub rms_dbfs: f64,
    pub clipped_samples: usize,
    /// (start_sec, end_sec) stretches quieter than -40 dBFS inside the piece.
    pub quiet_sections: Vec<(f64, f64)>,
    /// Energy shares 0..1: (low <150 Hz, mid 150-2000 Hz, high >2000 Hz).
    pub band_share: (f64, f64, f64),
}

fn dbfs(value: f64) -> f64 {
    if value <= 0.0 {
        -120.0
    } else {
        20.0 * value.log10()
    }
}

/// Decode a WAV file to mono f64 samples.
fn decode_mono(path: &Path) -> Result<(Vec<f64>, u32), String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("cannot open rendered WAV: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let raw: Vec<f64> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f64;
            reader
                .samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| s as f64 / max)
                .collect()
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .filter_map(Result::ok)
            .map(|s| s as f64)
            .collect(),
    };
    let mono: Vec<f64> = raw
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f64>() / channels as f64)
        .collect();
    Ok((mono, spec.sample_rate))
}

/// Analyze a rendered WAV file.
pub fn analyze_wav(path: &Path) -> Result<AudioReport, String> {
    let (samples, sample_rate) = decode_mono(path)?;
    if samples.is_empty() {
        return Err("rendered file contains no audio".into());
    }
    let duration_seconds = samples.len() as f64 / sample_rate as f64;

    let peak = samples.iter().fold(0f64, |m, s| m.max(s.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt();
    let clipped_samples = samples.iter().filter(|s| s.abs() >= 0.999).count();

    // Loudness curve in 250 ms windows; quiet = < -40 dBFS RMS. Trailing
    // silence (synth tail/end) is not reported.
    let window = (sample_rate as usize / 4).max(1);
    let mut quiet_sections = Vec::new();
    let mut current: Option<f64> = None;
    let windows: Vec<f64> = samples
        .chunks(window)
        .map(|chunk| dbfs((chunk.iter().map(|s| s * s).sum::<f64>() / chunk.len() as f64).sqrt()))
        .collect();
    let last_loud = windows.iter().rposition(|&db| db > -40.0).unwrap_or(0);
    for (i, &db) in windows.iter().enumerate().take(last_loud + 1) {
        let t = i as f64 * 0.25;
        if db < -40.0 {
            current.get_or_insert(t);
        } else if let Some(start) = current.take() {
            if t - start >= 0.5 {
                quiet_sections.push((start, t));
            }
        }
    }

    // Spectral balance: average FFT magnitude over 4096-sample windows.
    let fft_size = 4096.min(samples.len().next_power_of_two() / 2).max(256);
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(fft_size);
    let (mut low, mut mid, mut high) = (0f64, 0f64, 0f64);
    let hz_per_bin = sample_rate as f64 / fft_size as f64;
    for chunk in samples.chunks(fft_size).filter(|c| c.len() == fft_size) {
        let mut buffer: Vec<Complex<f64>> = chunk
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                // Hann window
                let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / fft_size as f64).cos();
                Complex::new(s * w, 0.0)
            })
            .collect();
        fft.process(&mut buffer);
        for (bin, value) in buffer.iter().take(fft_size / 2).enumerate() {
            let hz = bin as f64 * hz_per_bin;
            let energy = value.norm_sqr();
            if hz < 150.0 {
                low += energy;
            } else if hz < 2000.0 {
                mid += energy;
            } else {
                high += energy;
            }
        }
    }
    let total = (low + mid + high).max(f64::EPSILON);
    let band_share = (low / total, mid / total, high / total);

    Ok(AudioReport {
        duration_seconds,
        peak_dbfs: dbfs(peak),
        rms_dbfs: dbfs(rms),
        clipped_samples,
        quiet_sections,
        band_share,
    })
}

/// Producer's notes for the rendered mix.
pub fn describe(report: &AudioReport) -> String {
    let (low, mid, high) = report.band_share;
    let mut out = format!(
        "Rendered mix: {:.1}s, peak {:.1} dBFS, loudness {:.1} dBFS RMS\n\
         Spectral balance: {:.0}% low (<150Hz), {:.0}% mid, {:.0}% high (>2kHz)\n",
        report.duration_seconds,
        report.peak_dbfs,
        report.rms_dbfs,
        low * 100.0,
        mid * 100.0,
        high * 100.0,
    );
    if report.clipped_samples > 0 {
        out.push_str(&format!(
            "CLIPPING: {} samples at digital full scale — lower velocities or thin the arrangement\n",
            report.clipped_samples
        ));
    }
    if low > 0.55 {
        out.push_str(
            "LOW-END MUD: most energy sits under 150Hz — consider moving a part up an octave \
             or thinning simultaneous low notes\n",
        );
    }
    if high < 0.02 {
        out.push_str("Very dark mix: almost no energy above 2kHz (may be fine for the style)\n");
    }
    if report.quiet_sections.is_empty() {
        out.push_str("No unexpected quiet holes inside the piece.\n");
    } else {
        for (start, end) in report.quiet_sections.iter().take(5) {
            out.push_str(&format!(
                "QUIET HOLE: {:.2}s-{:.2}s is nearly silent — a part may be missing there\n",
                start, end
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a synthetic WAV: 1s of 440 Hz sine, 1s silence, 1s of 100 Hz sine.
    fn synth_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..44100 {
            let t = i as f64 / 44100.0;
            writer
                .write_sample(
                    (0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin() * 32767.0) as i16,
                )
                .unwrap();
        }
        for _ in 0..44100 {
            writer.write_sample(0i16).unwrap();
        }
        for i in 0..44100 {
            let t = i as f64 / 44100.0;
            writer
                .write_sample(
                    (0.5 * (2.0 * std::f64::consts::PI * 100.0 * t).sin() * 32767.0) as i16,
                )
                .unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn detects_duration_quiet_hole_and_low_energy() {
        let mut path = std::env::temp_dir();
        path.push(format!("tabmcp-audio-test-{}.wav", std::process::id()));
        synth_wav(&path);

        let report = analyze_wav(&path).expect("analyzes");
        std::fs::remove_file(&path).ok();

        assert!(
            (report.duration_seconds - 3.0).abs() < 0.05,
            "{}",
            report.duration_seconds
        );
        assert!(
            report.peak_dbfs > -7.0 && report.peak_dbfs < -5.0,
            "{}",
            report.peak_dbfs
        );
        assert_eq!(report.clipped_samples, 0);
        assert_eq!(
            report.quiet_sections.len(),
            1,
            "{:?}",
            report.quiet_sections
        );
        let (start, end) = report.quiet_sections[0];
        assert!(
            start > 0.8 && start < 1.3 && end > 1.7 && end < 2.2,
            "{start}..{end}"
        );
        // 100 Hz + 440 Hz sines: low and mid share energy, almost no high.
        let (low, _mid, high) = report.band_share;
        assert!(low > 0.3, "low share {low}");
        assert!(high < 0.05, "high share {high}");
        assert!(describe(&report).contains("QUIET HOLE"));
    }
}
