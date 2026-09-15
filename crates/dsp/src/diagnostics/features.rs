//! Spectral voice diagnostics (H-42, SPEC-007 §8.7): tone balance, sibilance and the de-esser
//! target, mains hum, and rumble — all read off a sine-normalized power spectrum
//! ([`super::spectrum`]) through band powers `Σ P[k] / ENBW`, so every number is a ratio that
//! doesn't depend on the FFT size or the window.
//!
//! Band edges and thresholds (SPEC-007 §8.7 — engineering defaults, not standards):
//! - overall level: 20 Hz … min(20 kHz, Nyquist);
//! - tone balance, **per-octave density** relative to the 1 kHz octave (707–1414 Hz):
//!   mud/boxiness 200–500 Hz, presence 2–5 kHz, air 10–16 kHz — pink noise reads 0 dB in each;
//! - sibilance 4–10 kHz, as a ratio to the overall level, with its centre frequency
//!   (the −3 dB span around the peak of the 1/3-octave-smoothed spectrum, geometric centre);
//! - rumble 20–80 Hz, as a ratio to the overall level;
//! - hum: harmonics 1…8 of 50 and 60 Hz, each compared with the median level between
//!   harmonics (±0.2 … ±0.5 × the mains frequency); prominent at ≥ 10 dB; hum is reported with
//!   ≥ 2 prominent harmonics, or one ≥ 20 dB among the first three. Needs bins ≤ 4 Hz wide.

use super::spectrum::power_db;

/// Overall level band (Hz).
pub const TOTAL_BAND_HZ: (f64, f64) = (20.0, 20_000.0);
/// Tone-balance reference: the 1 kHz octave (Hz).
pub const REFERENCE_BAND_HZ: (f64, f64) = (707.106_781, 1_414.213_562);
/// Mud / boxiness (Hz).
pub const MUD_BAND_HZ: (f64, f64) = (200.0, 500.0);
/// Presence (Hz).
pub const PRESENCE_BAND_HZ: (f64, f64) = (2000.0, 5000.0);
/// Air (Hz).
pub const AIR_BAND_HZ: (f64, f64) = (10_000.0, 16_000.0);
/// Sibilance (Hz).
pub const SIBILANCE_BAND_HZ: (f64, f64) = (4000.0, 10_000.0);
/// Rumble (Hz).
pub const RUMBLE_BAND_HZ: (f64, f64) = (20.0, 80.0);
/// Mains frequencies checked for hum (Hz).
pub const MAINS_HZ: [f64; 2] = [50.0, 60.0];
/// Harmonics of the mains frequency checked (1 = the fundamental).
pub const HUM_HARMONICS: u32 = 8;
/// A harmonic counts as prominent this far above its neighbourhood (dB).
pub const HUM_MIN_PROMINENCE_DB: f64 = 10.0;
/// A single harmonic (among the first three) this prominent is hum on its own (dB).
pub const HUM_SINGLE_LINE_DB: f64 = 20.0;
/// Hum detection needs bins at most this wide (Hz).
pub const HUM_MAX_BIN_HZ: f64 = 4.0;
/// Below this overall level (sine-referenced dB) nothing is diagnosed: silence.
pub const MIN_SIGNAL_DB: f64 = -90.0;
/// Relative values are clamped to ±this, so a silent band reads "very low" rather than −∞.
pub const RELATIVE_CLAMP_DB: f64 = 60.0;

/// A sine-normalized power spectrum and what's needed to read bands off it.
#[derive(Debug, Clone, Copy)]
pub struct SpectrumView<'a> {
    /// `fft_size / 2 + 1` bin powers.
    pub power: &'a [f64],
    /// Bin spacing, `fs / fft_size`.
    pub bin_hz: f64,
    /// The window's equivalent noise bandwidth, in bins.
    pub enbw_bins: f64,
}

impl<'a> SpectrumView<'a> {
    pub fn new(power: &'a [f64], sample_rate_hz: u32, fft_size: usize, enbw_bins: f64) -> Self {
        Self {
            power,
            bin_hz: f64::from(sample_rate_hz) / fft_size.max(1) as f64,
            enbw_bins,
        }
    }

    pub fn nyquist_hz(&self) -> f64 {
        self.bin_hz * self.power.len().saturating_sub(1) as f64
    }

    /// Bins whose centre lies in `[lo, hi)`.
    fn bins(&self, lo_hz: f64, hi_hz: f64) -> std::ops::Range<usize> {
        let start = (lo_hz / self.bin_hz).ceil().max(0.0) as usize;
        let end = ((hi_hz / self.bin_hz).ceil().max(0.0) as usize).min(self.power.len());
        start.min(end)..end
    }

    /// Band power `Σ P / ENBW` over `[lo, hi)`.
    pub fn band_power(&self, lo_hz: f64, hi_hz: f64) -> f64 {
        self.power[self.bins(lo_hz, hi_hz)].iter().sum::<f64>() / self.enbw_bins
    }

    /// `band` clipped to the spectrum; `None` when less than half an octave of it remains.
    fn clip(&self, band: (f64, f64)) -> Option<(f64, f64)> {
        let hi = band.1.min(self.nyquist_hz());
        (hi >= band.0 * std::f64::consts::SQRT_2).then_some((band.0, hi))
    }

    /// Overall level (dB, sine-referenced), `None` when the band is off the spectrum.
    pub fn total_db(&self) -> Option<f64> {
        let (lo, hi) = self.clip(TOTAL_BAND_HZ)?;
        Some(power_db(self.band_power(lo, hi)))
    }

    /// Per-octave density of `band` in dB (band level − 10·log10(octaves)).
    fn density_db(&self, band: (f64, f64)) -> Option<f64> {
        let (lo, hi) = self.clip(band)?;
        let octaves = (hi / lo).log2();
        Some(power_db(self.band_power(lo, hi)) - 10.0 * octaves.log10())
    }
}

fn clamp_relative(db: f64) -> f64 {
    if db.is_nan() {
        return -RELATIVE_CLAMP_DB;
    }
    db.clamp(-RELATIVE_CLAMP_DB, RELATIVE_CLAMP_DB)
}

/// Tone balance: per-octave densities relative to the 1 kHz octave (pink noise = 0 dB).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneBalance {
    pub mud_db: f64,
    pub presence_db: Option<f64>,
    /// `None` below a 32 kHz sample rate (no 10–16 kHz band to speak of).
    pub air_db: Option<f64>,
}

/// `None` for silence (the reference octave or the overall level below [`MIN_SIGNAL_DB`]).
pub fn tone_balance(view: &SpectrumView<'_>) -> Option<ToneBalance> {
    if view.total_db()? < MIN_SIGNAL_DB {
        return None;
    }
    let reference = view.density_db(REFERENCE_BAND_HZ)?;
    if !reference.is_finite() || reference < MIN_SIGNAL_DB {
        return None;
    }
    let rel = |band| view.density_db(band).map(|d| clamp_relative(d - reference));
    Some(ToneBalance {
        mud_db: rel(MUD_BAND_HZ)?,
        presence_db: rel(PRESENCE_BAND_HZ),
        air_db: rel(AIR_BAND_HZ),
    })
}

/// Sibilance: the 4–10 kHz band relative to the overall level, and its centre frequency (the
/// de-esser target).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sibilance {
    pub ratio_db: f64,
    pub centre_hz: f64,
}

/// `None` for silence, or when the spectrum stops below 8 kHz.
pub fn sibilance(view: &SpectrumView<'_>) -> Option<Sibilance> {
    let total = view.total_db()?;
    if total < MIN_SIGNAL_DB {
        return None;
    }
    let (lo, hi) = view.clip(SIBILANCE_BAND_HZ)?;
    if hi < 8000.0 {
        return None;
    }
    let band = view.band_power(lo, hi);
    if band <= 0.0 {
        return None;
    }
    Some(Sibilance {
        ratio_db: clamp_relative(power_db(band) - total),
        centre_hz: smoothed_centre_hz(view, lo, hi)?,
    })
}

/// Geometric centre of the −3 dB span around the maximum of the 1/3-octave-smoothed spectrum
/// within `[lo, hi)` (evaluated every 1/48 octave).
fn smoothed_centre_hz(view: &SpectrumView<'_>, lo: f64, hi: f64) -> Option<f64> {
    const STEPS_PER_OCTAVE: f64 = 48.0;
    const HALF_WIDTH_OCT: f64 = 1.0 / 6.0;
    let mut prefix = Vec::with_capacity(view.power.len() + 1);
    prefix.push(0.0);
    for &p in view.power {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + p);
    }
    let octaves = (hi / lo).log2();
    let steps = (octaves * STEPS_PER_OCTAVE).floor() as usize;
    let mut curve = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let f = lo * 2f64.powf(i as f64 / STEPS_PER_OCTAVE);
        let r = view.bins(
            f * 2f64.powf(-HALF_WIDTH_OCT),
            f * 2f64.powf(HALF_WIDTH_OCT),
        );
        let density = if r.is_empty() {
            0.0
        } else {
            (prefix[r.end] - prefix[r.start]) / r.len() as f64
        };
        curve.push((f, density));
    }
    let (peak_i, &(_, peak)) = curve
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.1.total_cmp(&b.1.1))?;
    if peak <= 0.0 {
        return None;
    }
    let half = peak / 2.0;
    let mut left = peak_i;
    while left > 0 && curve[left - 1].1 >= half {
        left -= 1;
    }
    let mut right = peak_i;
    while right + 1 < curve.len() && curve[right + 1].1 >= half {
        right += 1;
    }
    Some((curve[left].0 * curve[right].0).sqrt())
}

/// Mains hum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hum {
    /// 50 or 60 Hz.
    pub mains_hz: f64,
    /// Bit `h − 1` set when harmonic `h` (1 = the fundamental) stands out.
    pub harmonics: u32,
    /// The most prominent harmonic's frequency (Hz).
    pub strongest_hz: f64,
    /// Its prominence over the neighbourhood (dB).
    pub prominence_db: f64,
    /// Its level (dB, sine-referenced: a −40 dBFS-peak sine reads −40).
    pub level_db: f64,
}

struct HarmonicLine {
    freq_hz: f64,
    prominence_db: f64,
    level_db: f64,
}

fn harmonic_line(view: &SpectrumView<'_>, f: f64, mains: f64) -> Option<HarmonicLine> {
    let tol = (2.0 * view.bin_hz).max(0.006 * f);
    let lobe = view.bins(f - tol, f + tol + view.bin_hz * 0.5);
    if lobe.is_empty() {
        return None;
    }
    let (peak_bin, peak) = lobe
        .clone()
        .map(|k| (k, view.power[k]))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let mut flanks: Vec<f64> = view
        .bins(f - 0.5 * mains, f - 0.2 * mains)
        .chain(view.bins(f + 0.2 * mains, f + 0.5 * mains))
        .map(|k| power_db(view.power[k]))
        .filter(|d| d.is_finite())
        .collect();
    if flanks.len() < 3 || peak <= 0.0 {
        return None;
    }
    flanks.sort_by(f64::total_cmp);
    let baseline = flanks[flanks.len() / 2];
    let level = view.power[lobe].iter().sum::<f64>() / view.enbw_bins;
    Some(HarmonicLine {
        freq_hz: peak_bin as f64 * view.bin_hz,
        prominence_db: power_db(peak) - baseline,
        level_db: power_db(level),
    })
}

/// `None` when no mains hum stands out (or the bins are too wide to tell).
pub fn detect_hum(view: &SpectrumView<'_>) -> Option<Hum> {
    if view.bin_hz > HUM_MAX_BIN_HZ || view.total_db()? < MIN_SIGNAL_DB {
        return None;
    }
    let mut best: Option<(f64, Hum)> = None;
    for mains in MAINS_HZ {
        let mut mask = 0u32;
        let mut score = 0.0;
        let mut strongest: Option<HarmonicLine> = None;
        let mut single_line = false;
        for h in 1..=HUM_HARMONICS {
            let f = mains * f64::from(h);
            if f > view.nyquist_hz() * 0.9 {
                break;
            }
            let Some(line) = harmonic_line(view, f, mains) else {
                continue;
            };
            if line.prominence_db >= HUM_MIN_PROMINENCE_DB {
                mask |= 1 << (h - 1);
                score += line.prominence_db;
                if h <= 3 && line.prominence_db >= HUM_SINGLE_LINE_DB {
                    single_line = true;
                }
                if strongest
                    .as_ref()
                    .is_none_or(|s| line.prominence_db > s.prominence_db)
                {
                    strongest = Some(line);
                }
            }
        }
        let detected = mask.count_ones() >= 2 || single_line;
        if let (true, Some(line)) = (detected, strongest) {
            let hum = Hum {
                mains_hz: mains,
                harmonics: mask,
                strongest_hz: line.freq_hz,
                prominence_db: line.prominence_db,
                level_db: line.level_db,
            };
            if best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, hum));
            }
        }
    }
    best.map(|(_, hum)| hum)
}

/// Rumble: 20–80 Hz relative to the overall level (dB); `None` for silence.
pub fn rumble_db(view: &SpectrumView<'_>) -> Option<f64> {
    let total = view.total_db()?;
    if total < MIN_SIGNAL_DB {
        return None;
    }
    let (lo, hi) = view.clip(RUMBLE_BAND_HZ)?;
    Some(clamp_relative(power_db(view.band_power(lo, hi)) - total))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::spectrum::Ltas;
    use super::super::window::WindowKind;
    use super::*;
    use realfft::RealFftPlanner;

    const FS: u32 = 48_000;
    const FFT: usize = 16_384;

    /// Mean power spectrum (Hann, 16 384) of `x`.
    pub(crate) fn ltas(x: &[f32]) -> (Vec<f64>, f64) {
        let mut l = Ltas::new(FFT, WindowKind::Hann);
        let enbw = l.enbw_bins();
        l.push(x);
        (l.finish(), enbw)
    }

    fn view(p: &[f64], enbw: f64) -> SpectrumView<'_> {
        SpectrumView::new(p, FS, FFT, enbw)
    }

    /// White noise brick-wall band-limited to `[lo, hi)` Hz, scaled to `rms_dbfs`.
    pub(crate) fn band_noise(seed: u64, lo: f64, hi: f64, rms_dbfs: f64, seconds: f64) -> Vec<f32> {
        let n = ((seconds * f64::from(FS)) as usize).next_power_of_two();
        let white =
            vox_testkit::signal::white_noise(seed, 0.0, n as f64 / f64::from(FS), FS).unwrap();
        let mut planner = RealFftPlanner::<f64>::new();
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        let mut buf: Vec<f64> = white.iter().map(|&s| f64::from(s)).collect();
        buf.resize(n, 0.0);
        let mut spec = fwd.make_output_vec();
        fwd.process(&mut buf, &mut spec).unwrap();
        let bin_hz = f64::from(FS) / n as f64;
        for (k, c) in spec.iter_mut().enumerate() {
            let f = k as f64 * bin_hz;
            if f < lo || f >= hi {
                *c = realfft::num_complex::Complex::default();
            }
        }
        let mut out = inv.make_output_vec();
        inv.process(&mut spec, &mut out).unwrap();
        let rms = (out.iter().map(|v| v * v).sum::<f64>() / n as f64).sqrt();
        let target = 10f64.powf(rms_dbfs / 20.0);
        out.iter().map(|v| (v / rms * target) as f32).collect()
    }

    pub(crate) fn sine(freq_hz: f64, peak_dbfs: f64, seconds: f64) -> Vec<f32> {
        vox_testkit::signal::sine(freq_hz, peak_dbfs, seconds, FS).unwrap()
    }

    pub(crate) fn mix(parts: &[&[f32]]) -> Vec<f32> {
        let len = parts.iter().map(|p| p.len()).min().unwrap_or(0);
        (0..len).map(|i| parts.iter().map(|p| p[i]).sum()).collect()
    }

    #[test]
    fn pink_noise_has_a_flat_tone_balance() {
        let pink = vox_testkit::signal::pink_noise(1, -30.0, 20.0, FS).unwrap();
        let (p, enbw) = ltas(&pink);
        let tb = tone_balance(&view(&p, enbw)).expect("tone balance");
        assert!(tb.mud_db.abs() < 1.0, "mud {}", tb.mud_db);
        assert!(tb.presence_db.unwrap().abs() < 1.0);
        assert!(tb.air_db.unwrap().abs() < 1.0);
    }

    #[test]
    fn white_noise_tilts_up_three_db_per_octave() {
        // Density ∝ bandwidth: the geometric band centres sit −1.66 / +1.66 / +3.66 octaves
        // from 1 kHz, so +3.01 dB per octave gives −5.0 / +5.0 / +11.0 dB.
        let white = vox_testkit::signal::white_noise(2, -30.0, 20.0, FS).unwrap();
        let (p, enbw) = ltas(&white);
        let tb = tone_balance(&view(&p, enbw)).unwrap();
        assert!((tb.mud_db + 5.0).abs() < 1.0, "mud {}", tb.mud_db);
        assert!((tb.presence_db.unwrap() - 5.0).abs() < 1.0);
        assert!((tb.air_db.unwrap() - 11.0).abs() < 1.0);
    }

    #[test]
    fn sibilance_centre_on_band_limited_noise() {
        // Flat 5–8 kHz noise: the −3 dB span of the smoothed spectrum is the band itself, so
        // the target is its geometric centre, √(5000·8000) ≈ 6.32 kHz.
        let x = band_noise(3, 5000.0, 8000.0, -30.0, 10.0);
        let (p, enbw) = ltas(&x);
        let s = sibilance(&view(&p, enbw)).expect("sibilance");
        let oct = (s.centre_hz / 6324.6).log2().abs();
        assert!(oct < 1.0 / 6.0, "centre {} Hz", s.centre_hz);
        assert!(s.ratio_db > -0.5, "all energy is sibilance: {}", s.ratio_db);

        // A narrow 6.8–7.2 kHz band on top of a quieter broadband bed.
        let bed = vox_testkit::signal::pink_noise(4, -40.0, 10.0, FS).unwrap();
        let narrow = band_noise(5, 6800.0, 7200.0, -32.0, 10.0);
        let (p, enbw) = ltas(&mix(&[&bed, &narrow]));
        let s = sibilance(&view(&p, enbw)).unwrap();
        assert!(
            (s.centre_hz / 7000.0).log2().abs() < 1.0 / 12.0,
            "{}",
            s.centre_hz
        );
        // −32 dBFS narrow band over a −40 dBFS bed: the band holds almost all the energy.
        assert!((-2.0..=0.0).contains(&s.ratio_db), "ratio {}", s.ratio_db);
    }

    #[test]
    fn hum_at_50_and_60_hz_with_harmonics() {
        for mains in [50.0, 60.0] {
            let bed = vox_testkit::signal::pink_noise(6, -55.0, 10.0, FS).unwrap();
            let h1 = sine(mains, -40.0, 10.0);
            let h2 = sine(2.0 * mains, -46.0, 10.0);
            let h3 = sine(3.0 * mains, -43.0, 10.0);
            let (p, enbw) = ltas(&mix(&[&bed, &h1, &h2, &h3]));
            let hum = detect_hum(&view(&p, enbw)).expect("hum");
            assert!(
                (hum.mains_hz - mains).abs() < 1e-9,
                "mains {}",
                hum.mains_hz
            );
            assert_eq!(hum.harmonics & 0b111, 0b111, "mask {:b}", hum.harmonics);
            // "Strongest" = most prominent over its neighbourhood: over pink noise (louder at
            // low frequencies) that can be a harmonic rather than the fundamental. Its level is
            // the sine's own.
            let h = (hum.strongest_hz / mains).round();
            assert!(
                (hum.strongest_hz - h * mains).abs() < 3.0,
                "{}",
                hum.strongest_hz
            );
            let expected = [-40.0, -46.0, -43.0][h as usize - 1];
            assert!(
                (hum.level_db - expected).abs() < 1.0,
                "level {}",
                hum.level_db
            );
        }
        // Only the 2nd and 3rd harmonics (common with ground loops): still 50 Hz hum.
        let bed = vox_testkit::signal::pink_noise(7, -55.0, 10.0, FS).unwrap();
        let (p, enbw) = ltas(&mix(&[
            &bed,
            &sine(100.0, -45.0, 10.0),
            &sine(150.0, -48.0, 10.0),
        ]));
        let hum = detect_hum(&view(&p, enbw)).unwrap();
        assert!((hum.mains_hz - 50.0).abs() < 1e-9, "mains {}", hum.mains_hz);
        assert_eq!(hum.harmonics, 0b110);
    }

    #[test]
    fn no_hum_in_noise_or_an_unrelated_tone() {
        let pink = vox_testkit::signal::pink_noise(8, -40.0, 10.0, FS).unwrap();
        let (p, enbw) = ltas(&pink);
        assert!(detect_hum(&view(&p, enbw)).is_none());
        let tone = sine(440.0, -20.0, 10.0);
        let (p, enbw) = ltas(&mix(&[&pink, &tone]));
        assert!(detect_hum(&view(&p, enbw)).is_none());
        // Too coarse to tell: 8 192-point bins at 48 kHz are 5.9 Hz wide.
        let hum = mix(&[&pink, &sine(50.0, -30.0, 10.0), &sine(100.0, -30.0, 10.0)]);
        let mut l = Ltas::new(8192, WindowKind::Hann);
        l.push(&hum);
        let enbw = l.enbw_bins();
        let p = l.finish();
        assert!(detect_hum(&SpectrumView::new(&p, FS, 8192, enbw)).is_none());
    }

    #[test]
    fn rumble_share_of_pink_noise_and_a_low_tone() {
        // Pink noise: 2 of the 9.97 octaves between 20 Hz and 20 kHz → −7.0 dB.
        let pink = vox_testkit::signal::pink_noise(9, -30.0, 20.0, FS).unwrap();
        let (p, enbw) = ltas(&pink);
        let r = rumble_db(&view(&p, enbw)).unwrap();
        assert!((r + 6.98).abs() < 1.0, "rumble {r}");
        let (p, enbw) = ltas(&mix(&[&pink, &sine(40.0, -10.0, 20.0)]));
        assert!(rumble_db(&view(&p, enbw)).unwrap() > -1.0);
    }

    #[test]
    fn silence_diagnoses_nothing() {
        let p = vec![0.0; FFT / 2 + 1];
        let v = view(&p, 1.5);
        assert!(tone_balance(&v).is_none());
        assert!(sibilance(&v).is_none());
        assert!(detect_hum(&v).is_none());
        assert!(rumble_db(&v).is_none());
    }
}
