# Reference facts for spec writers

Collected 2026-09-12 by research agents. Items marked ⚠ were not independently verified — verify inside the spec ticket before relying on them.

## Loudness — ITU-R BS.1770-5 / EBU R128
Source: https://www.itu.int/dms_pubrec/itu-r/rec/bs/R-REC-BS.1770-5-202311-I!!PDF-E.pdf · https://tech.ebu.ch/docs/tech/tech3341v2_0.pdf · https://tech.ebu.ch/docs/tech/tech3342.pdf
- K-weighting at 48 kHz = two biquads.
  - Stage 1 (high shelf): b0=1.53512485958697, b1=−2.69169618940638, b2=1.19839281085285, a1=−1.69065929318241, a2=0.73248077421585
  - Stage 2 (RLB high-pass): b0=1.0, b1=−2.0, b2=1.0, a1=−1.99004745483398, a2=0.99007225036621
- Other sample rates: re-derive the analog prototypes (shelf ≈ +4 dB, fc ≈ 1681 Hz, Q ≈ 0.71; HP fc ≈ 38 Hz, Q ≈ 0.5) via bilinear transform (libebur128/pyloudnorm practice).
- Gating: 400 ms blocks, 75 % overlap; absolute gate −70 LUFS; relative gate −10 LU below the absolute-gated mean.
- Mono: channel weight G = 1.0 → L = −0.691 + 10·log10(mean square of K-weighted signal). A mono 1 kHz sine at −20 dBFS ≈ −23.0 LUFS.
- True peak (Annex 2): 4× oversampling interpolation, max |x|, in dBTP. ⚠ exact FIR taps not verified — use `ebur128` (`precision-true-peak`) and test it.
- EBU Tech 3341 examples (stereo): 1 kHz −23 dBFS/ch, 20 s → I = M = S = −23.0 ± 0.1 LUFS; −33 dBFS/ch → −33.0 ± 0.1 LUFS. ⚠ whether official test WAVs are downloadable is unverified → synthesize from the tables.
- LRA per Tech 3342 (10th–95th percentile of short-term loudness distribution, with gating).

## ACX (Audible) — https://help.acx.com/s/article/acx-audio-submission-requirements
- RMS between −23 dB and −18 dB; peak ≤ −3 dB; noise floor ≤ −60 dB RMS.
- MP3, ≥ 192 kbps CBR, 44.1 kHz; all files mono or all stereo.
- Room tone: 0.5–1 s at head, 1–5 s at tail; ≤ 120 min per file; one section per file.
- ⚠ Measurement weighting/windowing not specified by ACX. Common practice: unweighted RMS over the file; noise floor = quietest window (we use 500 ms, unweighted). Document our choice in SPEC-011.

## Adobe Audition reference behavior — https://helpx.adobe.com/audition/using/noise-reduction-restoration-effects.html · https://helpx.adobe.com/audition/using/amplitude-compression-effects.html
- Noise Reduction (process): Noise Reduction %, Reduce By (dB, ~10 typical), Spectral decay rate, Smoothing, Precision factor (5–10 recommended; < 3 blocky), Transition width (0 = hard per-band gate), FFT size; "Output noise only". Workflow: select room tone → Capture Noise Print.
- Dynamics: AutoGate, Compressor, Expander, Limiter sections; threshold/ratio per section; compressor attack/release/makeup; gain-reduction metering.
- Normalize: target dB or %, normalize all channels equally, DC bias adjust.
- ⚠ Parametric EQ exact band/Q/slope set unverified (helpx 403). Our design: HPF + LPF (6–48 dB/oct), low shelf, high shelf, 5 peaking bands.
- ⚠ Default shortcuts unverified: https://helpx.adobe.com/audition/desktop/keyboard-shortcuts/default-keyboard-shortcuts.html — verify in SPEC-019.

## Noise reduction algorithms
- Boll (1979), spectral subtraction, IEEE TASSP 27(2), doi:10.1109/TASSP.1979.1163209.
- Ephraim & Malah (1984) MMSE-STSA, IEEE TASSP 32(6); (1985) log-spectral amplitude, TASSP 33(2). Decision-directed a-priori SNR smoothing = standard musical-noise mitigation.
- Audacity Noise Reduction (open reference): Noise reduction dB, Sensitivity (0–24, default 6), Frequency smoothing bands — https://manual.audacityteam.org/man/noise_reduction.html

## Filters
- RBJ Audio EQ Cookbook (W3C note): https://www.w3.org/TR/audio-eq-cookbook/
- Butterworth cascade Q (Qk = 1/(2·cos θk)):
  - 12 dB/oct: 0.7071
  - 24 dB/oct: 0.5412, 1.3066
  - 36 dB/oct: 0.5177, 0.7071, 1.9319
  - 48 dB/oct: 0.5098, 0.6013, 0.9000, 2.5629
- 6 dB/oct = first-order section.

## Dither
- TPDF (sum of two independent uniform sources, ±1 LSB each) when reducing to 16/24-bit — best-practice convention (https://www.airwindows.com/tpdf-dither/).

## Ecosystem facts (see MEMORY.md gotchas for implications)
- cpal 0.18.2 · rtrb 0.4 · hound 3.5.1 · symphonia 0.6.1 (MPL-2.0) · flacenc 0.5.1 · LAME (LGPL-2.0-or-later) **loaded at runtime via `libloading`**; `mp3lame-encoder`/`mp3lame-sys` rejected because they always link LAME statically (ADR-007) · rubato 5.0 · realfft 3.5 · rustfft 6.4 · ebur128 0.1.10 · Tauri 2.11.5 · Svelte 5.57 · Vite 8.3 · Vitest 5.0 · svelte-check 4.7 · just 1.58.
- Plugins: clack (MIT/Apache), vst3 coupler-rs (MIT/Apache; VST3 SDK MIT since 3.8.0, Oct 2025), livi/lilv (ISC-style), ysfx JoepVanlier fork (library **Apache-2.0**; GPLv3 only for its JUCE plugin build, per ADR-007), VST2 = reverse-engineered headers only (legal risk). Versions seen 2026-09-12: CLAP 1.2.10, clack 0.2.0, VST3 SDK 3.8.1, LAME 4.0.
