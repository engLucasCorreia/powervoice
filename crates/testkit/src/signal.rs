//! Deterministic, seeded signal generators for DSP acceptance tests.

use crate::error::TestkitError;
use crate::prng::Pcg32;
use crate::units::dbfs_to_linear;

const SINE_PEAK_TO_RMS_DB: f64 = 3.0103; // 20*log10(sqrt(2))

fn num_frames(duration_s: f64, sample_rate: u32) -> usize {
    (duration_s * f64::from(sample_rate)).round() as usize
}

/// Common parameter validation shared by every generator: a generator needs
/// a positive sample rate and a positive duration to produce anything
/// meaningful.
fn validate_rate_and_duration(sample_rate: u32, duration_s: f64) -> Result<(), TestkitError> {
    if sample_rate == 0 {
        return Err(TestkitError::InvalidParameter(
            "sample_rate must be > 0".to_string(),
        ));
    }
    if duration_s.is_nan() || duration_s <= 0.0 {
        // Also rejects NaN, since `NaN > 0.0` is false.
        return Err(TestkitError::InvalidParameter(format!(
            "duration_s must be > 0, got {duration_s}"
        )));
    }
    Ok(())
}

/// A tone frequency must be positive and at or below Nyquist for
/// `sample_rate`, or it cannot be represented without aliasing.
fn validate_freq(freq_hz: f64, sample_rate: u32) -> Result<(), TestkitError> {
    if freq_hz.is_nan() || freq_hz <= 0.0 {
        return Err(TestkitError::InvalidParameter(format!(
            "freq_hz must be > 0, got {freq_hz}"
        )));
    }
    let nyquist_hz = f64::from(sample_rate) / 2.0;
    if freq_hz > nyquist_hz {
        return Err(TestkitError::InvalidParameter(format!(
            "freq_hz {freq_hz} exceeds Nyquist ({nyquist_hz} Hz at {sample_rate} Hz sample rate)"
        )));
    }
    Ok(())
}

/// Paul Kellet's refined pinking filter: a 7-pole IIR approximation of a
/// -3 dB/octave (pink) spectrum, designed for a 44.1 kHz sample rate (used
/// here at other rates too, as a "close enough" test signal rather than a
/// calibrated reference). Not the cheaper 3-coefficient "economy" variant
/// Kellet also published -- this is the higher-quality one, flat to within
/// about +/-0.5 dB from ~10 Hz-20 kHz at 44.1 kHz.
#[derive(Debug, Clone, Default)]
struct PinkFilter {
    b: [f64; 7],
}

impl PinkFilter {
    fn process(&mut self, white: f64) -> f64 {
        let b = &mut self.b;
        b[0] = 0.99886 * b[0] + white * 0.0555179;
        b[1] = 0.99332 * b[1] + white * 0.0750759;
        b[2] = 0.96900 * b[2] + white * 0.1538520;
        b[3] = 0.86650 * b[3] + white * 0.3104856;
        b[4] = 0.55000 * b[4] + white * 0.5329522;
        b[5] = -0.7616 * b[5] - white * 0.0168980;
        let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + white * 0.5362;
        b[6] = white * 0.115926;
        pink
    }
}

fn rms_of(samples: &[f64]) -> f64 {
    let n = samples.len().max(1);
    (samples.iter().map(|x| x * x).sum::<f64>() / n as f64).sqrt()
}

/// Rescales `raw` so its measured RMS equals `target_dbfs` exactly, then
/// casts to `f32`. Used so noise generators hit their target level exactly
/// rather than relying on a statistical approximation.
fn scale_to_rms(raw: &[f64], target_dbfs: f64) -> Vec<f32> {
    let actual_rms = rms_of(raw).max(1e-12);
    let target_rms = dbfs_to_linear(target_dbfs);
    let scale = target_rms / actual_rms;
    raw.iter().map(|&x| (x * scale) as f32).collect()
}

/// Generates a mono sine wave. `level_dbfs` is the peak amplitude of the
/// (continuous-time) sinusoid in dBFS -- for a pure tone this is also its
/// analytic true-peak level.
pub fn sine(
    freq_hz: f64,
    level_dbfs: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    sine_with_phase(freq_hz, level_dbfs, 0.0, duration_s, sample_rate)
}

/// Generates a mono sine wave with a phase offset in degrees.
///
/// Useful for building true-peak inter-sample-overshoot test signals: at
/// `freq_hz == sample_rate/4` with `phase_deg == 45.0`, every sample lands at
/// `±sqrt(2)/2` of the analytic peak, so the discrete sample peak reads
/// `3.0103 dB` below the analytic (true) peak.
pub fn sine_with_phase(
    freq_hz: f64,
    level_dbfs: f64,
    phase_deg: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    validate_rate_and_duration(sample_rate, duration_s)?;
    validate_freq(freq_hz, sample_rate)?;

    let amplitude = dbfs_to_linear(level_dbfs);
    let phase = phase_deg.to_radians();
    let n = num_frames(duration_s, sample_rate);
    let w = 2.0 * std::f64::consts::PI * freq_hz / f64::from(sample_rate);
    Ok((0..n)
        .map(|i| (amplitude * (w * i as f64 + phase).sin()) as f32)
        .collect())
}

/// Generates an interleaved stereo sine wave with the same level on both
/// channels (dual-mono), used for loudness conformance cases.
pub fn sine_stereo(
    freq_hz: f64,
    level_dbfs: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    let mono = sine(freq_hz, level_dbfs, duration_s, sample_rate)?;
    let mut out = Vec::with_capacity(mono.len() * 2);
    for s in mono {
        out.push(s);
        out.push(s);
    }
    Ok(out)
}

/// EBU Tech 3341 conformance case: interleaved stereo, both channels an
/// identical 1 kHz sine. Case 1 is -23 dBFS/ch (expect ~-23 LUFS I/M/S),
/// case 2 is -33 dBFS/ch (expect ~-33 LUFS).
pub fn tech3341_case(
    case: u8,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    let level_dbfs = match case {
        1 => -23.0,
        2 => -33.0,
        other => {
            return Err(TestkitError::InvalidParameter(format!(
                "unknown Tech 3341 case {other}, expected 1 or 2"
            )));
        }
    };
    sine_stereo(1000.0, level_dbfs, duration_s, sample_rate)
}

/// Logarithmic ("exponential") frequency sweep from `f_start_hz` to
/// `f_end_hz` over `duration_s`.
pub fn log_sweep(
    f_start_hz: f64,
    f_end_hz: f64,
    level_dbfs: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    validate_rate_and_duration(sample_rate, duration_s)?;
    validate_freq(f_start_hz, sample_rate)?;
    validate_freq(f_end_hz, sample_rate)?;

    let amplitude = dbfs_to_linear(level_dbfs);
    let n = num_frames(duration_s, sample_rate);
    let sr = f64::from(sample_rate);
    let k = (f_end_hz / f_start_hz).ln() / duration_s;
    Ok((0..n)
        .map(|i| {
            let t = i as f64 / sr;
            let phase = 2.0 * std::f64::consts::PI * f_start_hz * ((k * t).exp() - 1.0) / k;
            (amplitude * phase.sin()) as f32
        })
        .collect())
}

/// White noise with an exact target RMS level in dBFS, deterministic for a
/// given `seed`.
pub fn white_noise(
    seed: u64,
    level_dbfs: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    validate_rate_and_duration(sample_rate, duration_s)?;
    let n = num_frames(duration_s, sample_rate);
    let mut rng = Pcg32::new(seed, 0);
    let raw: Vec<f64> = (0..n).map(|_| rng.next_signed()).collect();
    Ok(scale_to_rms(&raw, level_dbfs))
}

/// Pink noise (Paul Kellet's refined pinking filter) with an exact target
/// RMS level in dBFS, deterministic for a given `seed`.
pub fn pink_noise(
    seed: u64,
    level_dbfs: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    validate_rate_and_duration(sample_rate, duration_s)?;
    let n = num_frames(duration_s, sample_rate);
    let mut rng = Pcg32::new(seed, 1);
    let mut filter = PinkFilter::default();
    let raw: Vec<f64> = (0..n).map(|_| filter.process(rng.next_signed())).collect();
    Ok(scale_to_rms(&raw, level_dbfs))
}

/// Digital silence.
pub fn silence(duration_s: f64, sample_rate: u32) -> Result<Vec<f32>, TestkitError> {
    validate_rate_and_duration(sample_rate, duration_s)?;
    Ok(vec![0.0; num_frames(duration_s, sample_rate)])
}

/// A single impulse of `level_dbfs` at `position_samples` (zero elsewhere).
pub fn impulse(
    level_dbfs: f64,
    position_samples: usize,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    validate_rate_and_duration(sample_rate, duration_s)?;
    let n = num_frames(duration_s, sample_rate);
    let mut out = vec![0.0f32; n];
    if position_samples < n {
        out[position_samples] = dbfs_to_linear(level_dbfs) as f32;
    }
    Ok(out)
}

/// Tone bursts: `on_s` seconds of a `freq_hz` tone at `level_dbfs` (peak),
/// alternating with `off_s` seconds of quiet, repeating for `duration_s`.
///
/// When `gap_noise_dbfs` is `Some`, the gaps are filled with white noise at
/// that exact RMS dBFS level (deterministic for `seed`) instead of pure
/// silence -- this is how the noise-floor acceptance fixture is built.
#[allow(clippy::too_many_arguments)]
pub fn tone_bursts(
    seed: u64,
    freq_hz: f64,
    level_dbfs: f64,
    gap_noise_dbfs: Option<f64>,
    on_s: f64,
    off_s: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    validate_rate_and_duration(sample_rate, duration_s)?;
    validate_freq(freq_hz, sample_rate)?;

    let n = num_frames(duration_s, sample_rate);
    let sr = f64::from(sample_rate);
    let amplitude = dbfs_to_linear(level_dbfs);
    let w = 2.0 * std::f64::consts::PI * freq_hz / sr;
    let on_samples = (on_s * sr).round() as usize;
    let off_samples = (off_s * sr).round() as usize;
    let period = (on_samples + off_samples).max(1);
    let is_gap = |i: usize| (i % period) >= on_samples;

    let mut gap_noise = if let Some(gap_db) = gap_noise_dbfs {
        let gap_count = (0..n).filter(|&i| is_gap(i)).count();
        let mut rng = Pcg32::new(seed, 2);
        let raw: Vec<f64> = (0..gap_count).map(|_| rng.next_signed()).collect();
        scale_to_rms(&raw, gap_db).into_iter()
    } else {
        Vec::new().into_iter()
    };

    Ok((0..n)
        .map(|i| {
            if is_gap(i) {
                gap_noise.next().unwrap_or(0.0)
            } else {
                (amplitude * (w * i as f64).sin()) as f32
            }
        })
        .collect())
}

/// A "voice-like" test signal: tone bursts (on/off pattern) layered over
/// continuous background noise at a given SNR (tone RMS vs. noise RMS, in
/// dB), deterministic for `seed`.
#[allow(clippy::too_many_arguments)]
pub fn voice_like(
    seed: u64,
    freq_hz: f64,
    tone_level_dbfs: f64,
    snr_db: f64,
    on_s: f64,
    off_s: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    validate_rate_and_duration(sample_rate, duration_s)?;
    validate_freq(freq_hz, sample_rate)?;

    let tone_rms_dbfs = tone_level_dbfs - SINE_PEAK_TO_RMS_DB;
    let noise_rms_dbfs = tone_rms_dbfs - snr_db;

    let mut out = white_noise(seed, noise_rms_dbfs, duration_s, sample_rate)?;
    let sr = f64::from(sample_rate);
    let amplitude = dbfs_to_linear(tone_level_dbfs);
    let w = 2.0 * std::f64::consts::PI * freq_hz / sr;
    let on_samples = (on_s * sr).round() as usize;
    let off_samples = (off_s * sr).round() as usize;
    let period = (on_samples + off_samples).max(1);
    for (i, sample) in out.iter_mut().enumerate() {
        if (i % period) < on_samples {
            *sample += (amplitude * (w * i as f64).sin()) as f32;
        }
    }
    Ok(out)
}

/// Streams a long fixture (pink noise background with periodic 1 kHz tone
/// bursts) directly to a WAV file at `path`, without holding the whole
/// signal in memory -- used for `fixtures/generated/long-60min-48k-mono.wav`.
pub fn write_long_fixture<P: AsRef<std::path::Path>>(
    path: P,
    duration_s: f64,
    sample_rate: u32,
    seed: u64,
) -> Result<(), TestkitError> {
    const NOISE_LEVEL_DBFS: f64 = -30.0;
    const TONE_LEVEL_DBFS: f64 = -18.0;
    const TONE_FREQ_HZ: f64 = 1000.0;
    const ON_S: f64 = 3.0;
    const OFF_S: f64 = 7.0;
    const CALIBRATION_S: f64 = 2.0;

    validate_rate_and_duration(sample_rate, duration_s)?;

    // Calibrate the pink-filter gain once on a short throwaway run so the
    // whole stream can be scaled with a single fixed factor: a second full
    // pass over 60 minutes is unnecessary, this filter's RMS is stable well
    // within a couple of seconds.
    let mut calib_rng = Pcg32::new(seed ^ 0x00C0_FFEE, 1);
    let mut calib_filter = PinkFilter::default();
    let calib_n = num_frames(CALIBRATION_S, sample_rate);
    let calib_raw: Vec<f64> = (0..calib_n)
        .map(|_| calib_filter.process(calib_rng.next_signed()))
        .collect();
    let calib_rms = rms_of(&calib_raw).max(1e-12);
    let noise_scale = dbfs_to_linear(NOISE_LEVEL_DBFS) / calib_rms;

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;

    let total_frames = num_frames(duration_s, sample_rate);
    let on_samples = (ON_S * f64::from(sample_rate)).round() as usize;
    let period_samples = (on_samples + (OFF_S * f64::from(sample_rate)).round() as usize).max(1);
    let tone_amp = dbfs_to_linear(TONE_LEVEL_DBFS);
    let w = 2.0 * std::f64::consts::PI * TONE_FREQ_HZ / f64::from(sample_rate);

    let mut rng = Pcg32::new(seed, 1);
    let mut filter = PinkFilter::default();
    for i in 0..total_frames {
        let pink = filter.process(rng.next_signed());
        let mut sample = pink * noise_scale;
        if (i % period_samples) < on_samples {
            sample += tone_amp * (w * i as f64).sin();
        }
        writer.write_sample(sample as f32)?;
    }
    writer.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_hits_target_peak() {
        let s = sine(1000.0, -20.0, 1.0, 48_000).unwrap();
        let peak = s.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!((crate::units::linear_to_dbfs(f64::from(peak)) - (-20.0)).abs() < 0.01);
    }

    #[test]
    fn generators_are_deterministic_for_a_seed() {
        let a = white_noise(1234, -20.0, 0.5, 48_000).unwrap();
        let b = white_noise(1234, -20.0, 0.5, 48_000).unwrap();
        assert_eq!(a, b);

        let a = pink_noise(999, -20.0, 0.5, 48_000).unwrap();
        let b = pink_noise(999, -20.0, 0.5, 48_000).unwrap();
        assert_eq!(a, b);

        let a = tone_bursts(5, 1000.0, -10.0, Some(-70.0), 0.2, 0.1, 1.0, 48_000).unwrap();
        let b = tone_bursts(5, 1000.0, -10.0, Some(-70.0), 0.2, 0.1, 1.0, 48_000).unwrap();
        assert_eq!(a, b);

        let a = voice_like(7, 300.0, -18.0, 10.0, 0.3, 0.2, 1.0, 48_000).unwrap();
        let b = voice_like(7, 300.0, -18.0, 10.0, 0.3, 0.2, 1.0, 48_000).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_diverge() {
        let a = white_noise(1, -20.0, 0.2, 48_000).unwrap();
        let b = white_noise(2, -20.0, 0.2, 48_000).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn white_noise_hits_target_rms() {
        let s = white_noise(42, -20.0, 2.0, 48_000).unwrap();
        assert!((crate::measure::rms_db(&s) - (-20.0)).abs() < 0.05);
    }

    #[test]
    fn pink_noise_hits_target_rms() {
        let s = pink_noise(42, -20.0, 2.0, 48_000).unwrap();
        assert!((crate::measure::rms_db(&s) - (-20.0)).abs() < 0.05);
    }

    #[test]
    fn tech3341_case_rejects_unknown_case() {
        assert!(tech3341_case(3, 1.0, 48_000).is_err());
    }

    #[test]
    fn rejects_non_positive_duration() {
        assert!(matches!(
            sine(1000.0, -20.0, 0.0, 48_000),
            Err(TestkitError::InvalidParameter(_))
        ));
        assert!(matches!(
            sine(1000.0, -20.0, -1.0, 48_000),
            Err(TestkitError::InvalidParameter(_))
        ));
        assert!(matches!(
            white_noise(1, -20.0, 0.0, 48_000),
            Err(TestkitError::InvalidParameter(_))
        ));
    }

    #[test]
    fn rejects_zero_sample_rate() {
        assert!(matches!(
            sine(1000.0, -20.0, 1.0, 0),
            Err(TestkitError::InvalidParameter(_))
        ));
    }

    #[test]
    fn rejects_non_positive_freq() {
        assert!(matches!(
            sine(0.0, -20.0, 1.0, 48_000),
            Err(TestkitError::InvalidParameter(_))
        ));
        assert!(matches!(
            sine(-100.0, -20.0, 1.0, 48_000),
            Err(TestkitError::InvalidParameter(_))
        ));
    }

    #[test]
    fn rejects_freq_above_nyquist() {
        assert!(matches!(
            sine(24_001.0, -20.0, 1.0, 48_000),
            Err(TestkitError::InvalidParameter(_))
        ));
        // freq_end above Nyquist for a sweep.
        assert!(matches!(
            log_sweep(20.0, 30_000.0, -20.0, 1.0, 48_000),
            Err(TestkitError::InvalidParameter(_))
        ));
    }
}
