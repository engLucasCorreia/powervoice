//! Minimal in-place radix-2 complex FFT (f64), for the §4.3 zipper harness only.
//!
//! Hand-written so `module-api` keeps its dependency list (ADR-005 §1) and so the harness never
//! grades production DSP (`dsp`) with itself (ADR-001 rule 2 spirit).

use std::f64::consts::PI;

/// Forward FFT of `re`/`im` in place. `re.len()` must be a power of two and equal `im.len()`.
pub(crate) fn fft_in_place(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(
        n.is_power_of_two() && im.len() == n,
        "fft: size must be a power of two"
    );
    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = -2.0 * PI / len as f64;
        for k in 0..half {
            let (s, c) = (step * k as f64).sin_cos();
            let mut start = 0;
            while start < n {
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                start += len;
            }
        }
        len *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_naive_dft() {
        let n = 64;
        let x: Vec<f64> = (0..n).map(|i| ((i * 7 % 13) as f64 - 6.0) * 0.1).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        fft_in_place(&mut re, &mut im);
        for k in 0..n {
            let (mut sr, mut si) = (0.0, 0.0);
            for (i, &v) in x.iter().enumerate() {
                let a = -2.0 * PI * (k * i) as f64 / n as f64;
                sr += v * a.cos();
                si += v * a.sin();
            }
            assert!(
                (sr - re[k]).abs() < 1e-9 && (si - im[k]).abs() < 1e-9,
                "bin {k}"
            );
        }
    }
}
