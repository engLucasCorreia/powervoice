//! Noise print: capture analysis (§4.1), blob v1 (§4.2), derivation (§4.3), describe (§4.10).

use std::sync::atomic::{AtomicBool, Ordering};

use realfft::RealFftPlanner;

/// Blob magic.
pub const BLOB_MAGIC: [u8; 4] = *b"PVNP";
/// Blob version this build writes (and the newest it reads).
pub const BLOB_VERSION: u16 = 1;
/// Header length in bytes.
pub const HEADER_LEN: usize = 48;
/// Analysis FFT size of a capture.
pub const CAPTURE_FFT_SIZE: usize = 8192;
/// Analysis hop of a capture.
pub const CAPTURE_HOP: usize = 2048;
/// Window id of the periodic Hann window.
pub const WINDOW_PERIODIC_HANN: u32 = 1;
/// Bins of a v1 capture (`N_c/2 + 1`).
pub const CAPTURE_BINS: usize = CAPTURE_FFT_SIZE / 2 + 1;
/// Size of a v1 blob: 48 + 8·4097 = 32 824 bytes.
pub const BLOB_LEN_V1: usize = HEADER_LEN + 8 * CAPTURE_BINS;
/// Minimum capture duration.
pub const MIN_CAPTURE_S: f64 = 0.5;
/// Minimum capture length in samples at any rate (3 analysis frames).
pub const MIN_CAPTURE_SAMPLES_ABS: usize = 12_288;
/// Only this much of an excerpt is analysed.
pub const MAX_CAPTURE_S: f64 = 60.0;
/// Equivalent noise bandwidth of the 4-term Blackman-Harris window (SPEC-007 analyzer), bins.
pub const ENBW_BH4_BINS: f64 = 2.0044;

const MIN_RATE_HZ: f64 = 8_000.0;
const MAX_RATE_HZ: f64 = 384_000.0;
const MIN_FFT: u32 = 1024;
const MAX_FFT: u32 = 16_384;
const CANCEL_POLL_FRAMES: usize = 64;
const LOG_EPS: f64 = 1e-30;
/// Level reported for analyzer bands without noise.
const EMPTY_BAND_DB: f32 = -150.0;

/// `max(round(0.5·fs), 12 288)` (SPEC-014 §2.3).
pub fn min_capture_samples(sample_rate: f64) -> usize {
    let half = (MIN_CAPTURE_S * sample_rate).round();
    let half = if half.is_finite() && half > 0.0 {
        half as usize
    } else {
        0
    };
    half.max(MIN_CAPTURE_SAMPLES_ABS)
}

/// `round(60·fs)`: the longest excerpt prefix [`capture_profile`] analyses.
pub fn max_capture_samples(sample_rate: f64) -> usize {
    let n = (MAX_CAPTURE_S * sample_rate).round();
    if n.is_finite() && n > 0.0 {
        n as usize
    } else {
        0
    }
}

/// Why a capture produced no blob.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CaptureError {
    /// The rate is outside 8 … 384 kHz.
    #[error("unsupported capture sample rate {0} Hz")]
    UnsupportedRate(f64),
    /// Shorter than [`min_capture_samples`].
    #[error("excerpt too short: {len} samples, at least {min} needed")]
    TooShort {
        /// Excerpt length.
        len: usize,
        /// Required length.
        min: usize,
    },
    /// Every sample is exactly 0.
    #[error("the excerpt is digital silence")]
    Silent,
    /// A sample is NaN or infinite.
    #[error("the excerpt contains non-finite samples")]
    NonFinite,
    /// `cancel` was set.
    #[error("capture cancelled")]
    Cancelled,
}

/// Analyses a noise-only excerpt (§4.1) and returns the blob v1 (§4.2).
///
/// Only the first 60 s are analysed. Frames: 8192-point periodic Hann, hop 2048, every frame
/// entirely inside the (capped) excerpt. Per bin: mean power density `D(k)` and log-power spread
/// `S(k)` (population standard deviation, dB), accumulated in f64. `cancel` is polled every 64
/// frames. Deterministic: the same excerpt and rate give byte-identical blobs (same machine).
///
/// # Errors
/// See [`CaptureError`].
pub fn capture_profile(
    x: &[f32],
    sample_rate: f64,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, CaptureError> {
    if !(sample_rate.is_finite() && (MIN_RATE_HZ..=MAX_RATE_HZ).contains(&sample_rate)) {
        return Err(CaptureError::UnsupportedRate(sample_rate));
    }
    let x = &x[..x.len().min(max_capture_samples(sample_rate))];
    let min = min_capture_samples(sample_rate);
    if x.len() < min {
        return Err(CaptureError::TooShort { len: x.len(), min });
    }
    if x.iter().any(|s| !s.is_finite()) {
        return Err(CaptureError::NonFinite);
    }
    if x.iter().all(|&s| s == 0.0) {
        return Err(CaptureError::Silent);
    }

    let n = CAPTURE_FFT_SIZE;
    let frames = (x.len() - n) / CAPTURE_HOP + 1;
    let window: Vec<f64> = (0..n)
        .map(|i| 0.5 * (1.0 - (std::f64::consts::TAU * i as f64 / n as f64).cos()))
        .collect();
    // Σ w² of the periodic Hann window: exactly 3N/8.
    let sum_w2 = 3.0 * n as f64 / 8.0;
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(n);
    let mut buf = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let mut power_sum = vec![0.0f64; CAPTURE_BINS];
    let mut log_mean = vec![0.0f64; CAPTURE_BINS];
    let mut log_m2 = vec![0.0f64; CAPTURE_BINS];
    for j in 0..frames {
        if j % CANCEL_POLL_FRAMES == 0 && cancel.load(Ordering::Relaxed) {
            return Err(CaptureError::Cancelled);
        }
        let start = j * CAPTURE_HOP;
        for (b, (&s, &w)) in buf.iter_mut().zip(x[start..start + n].iter().zip(&window)) {
            *b = f64::from(s) * w;
        }
        // Buffer lengths come from the plan, so this cannot fail.
        let _ = fft.process_with_scratch(&mut buf, &mut spec, &mut scratch);
        let count = (j + 1) as f64;
        for (k, c) in spec.iter().enumerate() {
            let p = c.norm_sqr() / sum_w2;
            power_sum[k] += p;
            // Welford's running mean/variance of the log power.
            let l = 10.0 * (p + LOG_EPS).log10();
            let delta = l - log_mean[k];
            log_mean[k] += delta / count;
            log_m2[k] += delta * (l - log_mean[k]);
        }
    }
    let m = frames as f64;
    let density: Vec<f32> = power_sum.iter().map(|&s| (s / m) as f32).collect();
    let spread: Vec<f32> = log_m2
        .iter()
        .map(|&v| (v / m).max(0.0).sqrt() as f32)
        .collect();
    let energy: f64 = x.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let rms_dbfs = (10.0 * (energy / x.len() as f64).log10()) as f32;

    let mut b = Vec::with_capacity(BLOB_LEN_V1);
    b.extend_from_slice(&BLOB_MAGIC);
    b.extend_from_slice(&BLOB_VERSION.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(n as u32).to_le_bytes());
    b.extend_from_slice(&(CAPTURE_HOP as u32).to_le_bytes());
    b.extend_from_slice(&WINDOW_PERIODIC_HANN.to_le_bytes());
    b.extend_from_slice(&(frames as u32).to_le_bytes());
    b.extend_from_slice(&(x.len() as u64).to_le_bytes());
    b.extend_from_slice(&rms_dbfs.to_le_bytes());
    b.extend_from_slice(&(CAPTURE_BINS as u32).to_le_bytes());
    for v in density.iter().chain(&spread) {
        b.extend_from_slice(&v.to_le_bytes());
    }
    debug_assert_eq!(b.len(), BLOB_LEN_V1);
    Ok(b)
}

/// Why a blob was rejected (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BlobError {
    /// Wrong magic, truncated, inconsistent or holding a non-finite/negative value.
    #[error("invalid noise print: {0}")]
    Invalid(&'static str),
    /// Written by a newer PowerVoice (blob version > 1).
    #[error("noise print version {0} is newer than supported")]
    TooNew(u16),
}

fn rd<const W: usize>(b: &[u8], at: usize) -> [u8; W] {
    let mut a = [0u8; W];
    a.copy_from_slice(&b[at..at + W]);
    a
}

/// A validated blob. Parsing and every accessor are allocation-free.
#[derive(Clone, Copy, Debug)]
pub struct ProfileView<'a> {
    bytes: &'a [u8],
    sample_rate_hz: f64,
    fft_size: usize,
    hop: u32,
    window_id: u32,
    frames: u32,
    excerpt_len: u64,
    rms_dbfs: f32,
    bins: usize,
}

impl<'a> ProfileView<'a> {
    /// Validates `bytes` (§4.2): magic, version, `N_c` a power of two in 1024 … 16384,
    /// `B = N_c/2 + 1`, rate in 8 … 384 kHz, exact length, every value finite and `D, S ≥ 0`.
    ///
    /// # Errors
    /// [`BlobError::TooNew`] for a version > 1, [`BlobError::Invalid`] otherwise.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, BlobError> {
        if bytes.len() < 8 || bytes[..4] != BLOB_MAGIC {
            return Err(BlobError::Invalid("wrong magic or truncated"));
        }
        let version = u16::from_le_bytes(rd(bytes, 4));
        if version > BLOB_VERSION {
            return Err(BlobError::TooNew(version));
        }
        if version == 0 {
            return Err(BlobError::Invalid("version 0"));
        }
        if bytes.len() < HEADER_LEN {
            return Err(BlobError::Invalid("truncated header"));
        }
        let sample_rate_hz = f64::from_le_bytes(rd(bytes, 8));
        let fft_size = u32::from_le_bytes(rd(bytes, 16));
        let bins = u32::from_le_bytes(rd(bytes, 44)) as usize;
        if !(fft_size.is_power_of_two() && (MIN_FFT..=MAX_FFT).contains(&fft_size)) {
            return Err(BlobError::Invalid("analysis FFT size"));
        }
        if bins != fft_size as usize / 2 + 1 {
            return Err(BlobError::Invalid("bin count"));
        }
        if !(sample_rate_hz.is_finite() && (MIN_RATE_HZ..=MAX_RATE_HZ).contains(&sample_rate_hz)) {
            return Err(BlobError::Invalid("sample rate"));
        }
        let rms_dbfs = f32::from_le_bytes(rd(bytes, 40));
        if !rms_dbfs.is_finite() {
            return Err(BlobError::Invalid("excerpt RMS"));
        }
        if bytes.len() != HEADER_LEN + 8 * bins {
            return Err(BlobError::Invalid("length"));
        }
        if bytes[HEADER_LEN..]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|c| !matches!(f32::from_le_bytes(*c), v if v.is_finite() && v >= 0.0))
        {
            return Err(BlobError::Invalid("non-finite or negative value"));
        }
        Ok(Self {
            bytes,
            sample_rate_hz,
            fft_size: fft_size as usize,
            hop: u32::from_le_bytes(rd(bytes, 20)),
            window_id: u32::from_le_bytes(rd(bytes, 24)),
            frames: u32::from_le_bytes(rd(bytes, 28)),
            excerpt_len: u64::from_le_bytes(rd(bytes, 32)),
            rms_dbfs,
            bins,
        })
    }

    /// Capture sample rate `fs_c`.
    pub fn sample_rate_hz(&self) -> f64 {
        self.sample_rate_hz
    }
    /// Analysis FFT size `N_c`.
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }
    /// Analysis hop.
    pub fn hop(&self) -> u32 {
        self.hop
    }
    /// Window id (1 = periodic Hann).
    pub fn window_id(&self) -> u32 {
        self.window_id
    }
    /// Analysed frames `M`.
    pub fn frames(&self) -> u32 {
        self.frames
    }
    /// Analysed excerpt length (after the 60 s cap).
    pub fn excerpt_len(&self) -> u64 {
        self.excerpt_len
    }
    /// Excerpt RMS, dBFS.
    pub fn rms_dbfs(&self) -> f32 {
        self.rms_dbfs
    }
    /// Bins `B`.
    pub fn bins(&self) -> usize {
        self.bins
    }
    /// Mean power density `D(k)` (linear; white noise of variance σ² gives σ²).
    pub fn density(&self, k: usize) -> f32 {
        f32::from_le_bytes(rd(self.bytes, HEADER_LEN + 4 * k))
    }
    /// Log-power spread `S(k)` in dB.
    pub fn spread(&self, k: usize) -> f32 {
        f32::from_le_bytes(rd(self.bytes, HEADER_LEN + 4 * (self.bins + k)))
    }
}

/// §4.3 density model: piecewise-linear through the capture bin centres, flat above the capture
/// Nyquist at the mean of the top 1/3 octave.
struct DensityModel<'v, 'a> {
    view: &'v ProfileView<'a>,
    spacing_hz: f64,
    nyquist_hz: f64,
    top_mean: f64,
}

impl<'v, 'a> DensityModel<'v, 'a> {
    fn new(view: &'v ProfileView<'a>) -> Self {
        let last = view.bins() - 1;
        let spacing_hz = view.sample_rate_hz() / view.fft_size() as f64;
        let first_top = ((last as f64 * 2f64.powf(-1.0 / 3.0)).ceil() as usize).min(last);
        let top: f64 = (first_top..=last).map(|k| f64::from(view.density(k))).sum();
        Self {
            view,
            spacing_hz,
            nyquist_hz: last as f64 * spacing_hz,
            top_mean: top / (last - first_top + 1) as f64,
        }
    }

    fn d(&self, k: usize) -> f64 {
        f64::from(self.view.density(k))
    }

    /// ∫ of the linear interpolation over `[a, b]` Hz, `0 ≤ a < b ≤ Nyquist`.
    fn integral_linear(&self, a: f64, b: f64) -> f64 {
        let last = self.view.bins() - 1;
        let (u0, u1) = (a / self.spacing_hz, b / self.spacing_hz);
        let mut i = (u0.floor() as usize).min(last - 1);
        let mut s0 = u0;
        let mut acc = 0.0;
        while s0 < u1 && i < last {
            let s1 = u1.min((i + 1) as f64);
            if s1 > s0 {
                let (d0, d1) = (self.d(i), self.d(i + 1));
                let at = |s: f64| d0 + (s - i as f64) * (d1 - d0);
                acc += (s1 - s0) * 0.5 * (at(s0) + at(s1));
            }
            s0 = s1;
            i += 1;
        }
        acc * self.spacing_hz
    }

    /// Mean of the model over `[lo, hi]` Hz (`0 ≤ lo`).
    fn mean(&self, lo: f64, hi: f64) -> f64 {
        if hi <= lo {
            return 0.0;
        }
        let mut acc = 0.0;
        if lo < self.nyquist_hz {
            acc += self.integral_linear(lo, hi.min(self.nyquist_hz));
        }
        if hi > self.nyquist_hz {
            acc += self.top_mean * (hi - lo.max(self.nyquist_hz));
        }
        acc / (hi - lo)
    }
}

/// Noise power per processing bin `λ(k')`, `k' = 0 … N/2`, for FFT size `fft_size` at
/// `sample_rate` (§4.3): the density model averaged over each bin's band (clipped to
/// `[0, fs/2]`), scaled by `fs/fs_c`, times `Σ w_a² = N/2` (√Hann). With `fs = fs_c` and
/// `N = N_c` it is the identity. Allocates (call from `activate`).
pub fn derive_lambda(view: &ProfileView<'_>, fft_size: usize, sample_rate: f64) -> Vec<f32> {
    let bins = fft_size / 2 + 1;
    let sum_w2 = fft_size as f64 / 2.0;
    if fft_size == view.fft_size() && sample_rate.to_bits() == view.sample_rate_hz().to_bits() {
        return (0..bins)
            .map(|k| (f64::from(view.density(k)) * sum_w2) as f32)
            .collect();
    }
    let model = DensityModel::new(view);
    let ratio = sample_rate / view.sample_rate_hz();
    let spacing = sample_rate / fft_size as f64;
    let nyquist = sample_rate / 2.0;
    (0..bins)
        .map(|k| {
            let f = k as f64 * spacing;
            let lo = (f - spacing / 2.0).max(0.0);
            let hi = (f + spacing / 2.0).min(nyquist);
            (ratio * model.mean(lo, hi) * sum_w2) as f32
        })
        .collect()
}

/// The print on the SPEC-007 analyzer bands (§4.10): one `(f_k, L_k)` point per band centre
/// `f_k = 20·2^(k/24)` Hz up to `min(fs_c/2, 24 kHz)`, `L_k` in the analyzer's convention
/// (white noise of σ dBFS RMS at 48 kHz reads σ − 30.09 dB). Clears `out`; allocates nothing
/// else.
pub fn describe_points(view: &ProfileView<'_>, out: &mut Vec<(f32, f32)>) {
    out.clear();
    let fs_c = view.sample_rate_hz();
    let f_max = (fs_c / 2.0).min(24_000.0);
    let n_an = 2f64.powf((8192.0 * fs_c / 48_000.0).log2().round());
    let offset_db = 10.0 * (4.0 * ENBW_BH4_BINS / n_an).log10();
    let model = DensityModel::new(view);
    let half_band = 2f64.powf(1.0 / 48.0);
    for k in 0.. {
        let f = 20.0 * 2f64.powf(f64::from(k) / 24.0);
        if f > f_max {
            break;
        }
        let d = model.mean(f / half_band, f * half_band);
        let level = if d > 0.0 {
            (10.0 * d.log10() + offset_db) as f32
        } else {
            EMPTY_BAND_DB
        };
        out.push((f as f32, level));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A v1-shaped blob with density `d(k)` and spread 0 at `fs`.
    fn blob_with(fs: f64, d: impl Fn(usize) -> f32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&BLOB_MAGIC);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&fs.to_le_bytes());
        b.extend_from_slice(&8192u32.to_le_bytes());
        b.extend_from_slice(&2048u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&12_288u64.to_le_bytes());
        b.extend_from_slice(&(-50.0f32).to_le_bytes());
        b.extend_from_slice(&4097u32.to_le_bytes());
        for k in 0..CAPTURE_BINS {
            b.extend_from_slice(&d(k).to_le_bytes());
        }
        for _ in 0..CAPTURE_BINS {
            b.extend_from_slice(&0f32.to_le_bytes());
        }
        b
    }

    /// Seeded uniform noise in [-1, 1) (xorshift64*).
    fn noise(seed: u64, n: usize, amp: f32) -> Vec<f32> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s ^= s >> 12;
                s ^= s << 25;
                s ^= s >> 27;
                let u = (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
                amp * (2.0 * u - 1.0) as f32
            })
            .collect()
    }

    #[test]
    fn constants() {
        assert_eq!(BLOB_LEN_V1, 32_824);
        assert_eq!(min_capture_samples(48_000.0), 24_000);
        assert_eq!(min_capture_samples(44_100.0), 22_050);
        assert_eq!(min_capture_samples(96_000.0), 48_000);
        assert_eq!(min_capture_samples(16_000.0), 12_288);
        assert_eq!(max_capture_samples(48_000.0), 2_880_000);
    }

    #[test]
    fn same_rate_identity_and_half_weighted_box() {
        let d = |k: usize| 1e-6 * (1.0 + (k as f32 * 0.37).sin().abs() + (k % 7) as f32);
        let blob = blob_with(48_000.0, d);
        let v = ProfileView::parse(&blob).unwrap();
        // r = 1: identity (× Σw² = N/2).
        let l = derive_lambda(&v, 8192, 48_000.0);
        assert_eq!(l.len(), CAPTURE_BINS);
        for (k, &got) in l.iter().enumerate() {
            assert_eq!(got.to_bits(), ((f64::from(d(k)) * 4096.0) as f32).to_bits());
        }
        // Even r: exactly the half-weighted box average, mirrored at the ends.
        for n in [1024usize, 2048, 4096] {
            let r = 8192 / n;
            let l = derive_lambda(&v, n, 48_000.0);
            let dm = |i: i64| -> f64 {
                let last = CAPTURE_BINS as i64 - 1;
                let i = if i < 0 {
                    -i
                } else if i > last {
                    2 * last - i
                } else {
                    i
                };
                f64::from(d(i as usize))
            };
            for (kp, &got) in l.iter().enumerate() {
                let c = (r * kp) as i64;
                let h = (r / 2) as i64;
                let mut s = 0.5 * dm(c - h) + 0.5 * dm(c + h);
                for i in (1 - h)..h {
                    s += dm(c + i);
                }
                let want = s / r as f64 * n as f64 / 2.0;
                let rel = (f64::from(got) - want).abs() / want;
                assert!(rel < 1e-5, "N {n} bin {kp}: {got} vs {want}");
            }
        }
    }

    #[test]
    fn flat_density_scales_with_the_rate() {
        let sigma2 = 1e-5f32;
        let blob = blob_with(48_000.0, |_| sigma2);
        let v = ProfileView::parse(&blob).unwrap();
        for (fs, n) in [(44_100.0, 2048usize), (96_000.0, 1024), (48_000.0, 4096)] {
            let l = derive_lambda(&v, n, fs);
            let want = f64::from(sigma2) * fs / 48_000.0 * n as f64 / 2.0;
            for (k, &got) in l.iter().enumerate() {
                assert!(
                    (f64::from(got) / want - 1.0).abs() < 1e-5,
                    "{fs} Hz N {n} bin {k}: {got} vs {want}"
                );
            }
        }
    }

    #[test]
    fn capture_of_white_noise() {
        let fs = 48_000.0;
        let amp = 0.01f32;
        let x = noise(7, 480_000, amp);
        let sigma2 = f64::from(amp) * f64::from(amp) / 3.0;
        let blob = capture_profile(&x, fs, &AtomicBool::new(false)).unwrap();
        assert_eq!(blob.len(), BLOB_LEN_V1);
        let v = ProfileView::parse(&blob).unwrap();
        assert_eq!(v.frames(), 231);
        assert_eq!(v.excerpt_len(), 480_000);
        let (mut d_sum, mut s_sum) = (0.0, 0.0);
        for k in 100..4000 {
            d_sum += f64::from(v.density(k));
            s_sum += f64::from(v.spread(k));
        }
        let d_mean = d_sum / 3900.0;
        let s_mean = s_sum / 3900.0;
        assert!(
            (10.0 * (d_mean / sigma2).log10()).abs() < 0.1,
            "{d_mean} vs {sigma2}"
        );
        assert!((s_mean - 5.57).abs() < 0.3, "spread {s_mean}");
        let rms = 10.0 * sigma2.log10();
        assert!((f64::from(v.rms_dbfs()) - rms).abs() < 0.05);

        let mut pts = Vec::new();
        describe_points(&v, &mut pts);
        assert_eq!(pts.len(), 246);
        // Analyzer convention: σ dBFS white noise reads σ − 30.09 dB. Bands below ~1 kHz are
        // narrower than a few capture bins, so single bands scatter by ~±0.5 dB (10 s print):
        // check the 1–20 kHz mean tightly and every 100 Hz–20 kHz band loosely.
        let want = rms - 30.09;
        let hi: Vec<f64> = pts
            .iter()
            .filter(|p| (1_000.0..=20_000.0).contains(&p.0))
            .map(|p| f64::from(p.1))
            .collect();
        let mean = hi.iter().sum::<f64>() / hi.len() as f64;
        assert!((mean - want).abs() < 0.1, "1–20 kHz mean {mean} vs {want}");
        for &(f, l) in &pts {
            if (100.0..=20_000.0).contains(&f) {
                assert!((f64::from(l) - want).abs() < 1.5, "{f} Hz: {l}");
            }
        }
    }

    #[test]
    fn capture_errors() {
        let no = AtomicBool::new(false);
        let x = noise(1, 24_000, 0.1);
        assert!(matches!(
            capture_profile(&x[..23_999], 48_000.0, &no),
            Err(CaptureError::TooShort { .. })
        ));
        assert!(capture_profile(&x, 48_000.0, &no).is_ok());
        assert_eq!(
            capture_profile(&[0.0; 24_000], 48_000.0, &no),
            Err(CaptureError::Silent)
        );
        assert_eq!(
            capture_profile(&x, 48_000.0, &AtomicBool::new(true)),
            Err(CaptureError::Cancelled)
        );
        assert!(matches!(
            capture_profile(&x, 4_000.0, &no),
            Err(CaptureError::UnsupportedRate(_))
        ));
        let mut bad = x.clone();
        bad[5] = f32::NAN;
        assert_eq!(
            capture_profile(&bad, 48_000.0, &no),
            Err(CaptureError::NonFinite)
        );
    }

    #[test]
    fn blob_validation() {
        let good = blob_with(48_000.0, |_| 1e-6);
        assert!(ProfileView::parse(&good).is_ok());
        let mut b = good.clone();
        b[0] = b'X';
        assert!(matches!(ProfileView::parse(&b), Err(BlobError::Invalid(_))));
        assert!(matches!(
            ProfileView::parse(&good[..good.len() - 1]),
            Err(BlobError::Invalid(_))
        ));
        assert!(matches!(
            ProfileView::parse(&[]),
            Err(BlobError::Invalid(_))
        ));
        for bad in [f32::NAN, -1.0, f32::INFINITY] {
            let mut b = good.clone();
            b[HEADER_LEN + 4 * 10..HEADER_LEN + 4 * 11].copy_from_slice(&bad.to_le_bytes());
            assert!(matches!(ProfileView::parse(&b), Err(BlobError::Invalid(_))));
        }
        let mut b = good.clone();
        b[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(ProfileView::parse(&b).unwrap_err(), BlobError::TooNew(2));
        let mut b = good.clone();
        b[8..16].copy_from_slice(&1_000.0f64.to_le_bytes());
        assert!(matches!(ProfileView::parse(&b), Err(BlobError::Invalid(_))));
        let mut b = good;
        b[44..48].copy_from_slice(&4096u32.to_le_bytes());
        assert!(matches!(ProfileView::parse(&b), Err(BlobError::Invalid(_))));
    }
}
