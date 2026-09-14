//! Multichannel -> mono downmix (SPEC-005 §2.4, §4.3) and the channel-probe classification the
//! open dialog needs (T-209 builds the dialog; this module only produces the data it needs).

/// One channel's role, from the container's channel mask (or a positional guess for a channel
/// count with no mask — SPEC-005 §2.4's "Channel N" fallback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    /// A display label: "Left", "Right", "Center", "LFE", "Surround Left", …, or "Channel N"
    /// (SPEC-005 §2.4).
    pub label: String,
    /// Excluded from [`downmix_average`] (SPEC-005 §2.4, §4.3).
    pub is_lfe: bool,
}

/// How multichannel input becomes the mono document (SPEC-005 §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownmixChoice {
    /// Average of every non-LFE channel (default).
    Average,
    /// Copy one channel verbatim (0-based index into the source's channel list).
    Channel(usize),
}

/// SPEC-005 §2.4 thresholds for the silent/active channel hint (`silent_channel_dbfs`).
pub const SILENT_CHANNEL_DBFS: f32 = -70.0;
pub const ACTIVE_CHANNEL_DBFS: f32 = -50.0;

/// Averages one interleaved frame's non-LFE channels (SPEC-005 §4.3, normative for AC-9):
/// `y = (Σ x_c) · (1/n)` computed in `f32`, summed in channel order, LFE excluded from both the
/// sum and `n`. For n = 2 the factor is exactly `0.5`; for other n it is `1.0f32 / n as f32`,
/// matching what a caller re-deriving `(sum) * (1/n as f32)` independently would get bit-exactly
/// (AC-9e).
///
/// `frame.len()` must equal `lfe.len()` (the original channel count); a channel past the end of
/// `lfe` is treated as non-LFE.
pub fn downmix_average(frame: &[f32], lfe: &[bool]) -> f32 {
    let mut sum = 0.0f32;
    let mut n: u32 = 0;
    for (i, &s) in frame.iter().enumerate() {
        if !lfe.get(i).copied().unwrap_or(false) {
            sum += s;
            n += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    let factor = 1.0f32 / n as f32;
    sum * factor
}

/// Copies one channel verbatim (SPEC-005 §2.4 "pick a channel").
pub fn downmix_pick(frame: &[f32], index: usize) -> f32 {
    frame.get(index).copied().unwrap_or(0.0)
}

/// Applies `choice` to one interleaved frame.
pub fn downmix_frame(frame: &[f32], lfe: &[bool], choice: DownmixChoice) -> f32 {
    match choice {
        DownmixChoice::Average => downmix_average(frame, lfe),
        DownmixChoice::Channel(index) => downmix_pick(frame, index),
    }
}

/// Per-channel peak found by the probe window (SPEC-005 §2.4's silent-channel hint), in dBFS
/// (`f32::NEG_INFINITY` for exact digital silence).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelPeak {
    pub peak_dbfs: f32,
}

fn peak_to_dbfs(peak: f32) -> f32 {
    if peak <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * peak.log10()
    }
}

/// Accumulates per-channel sample peaks over the probe window (SPEC-005 §2.4/§4.3: "the probe...
/// keeps a per-channel peak").
#[derive(Debug, Clone)]
pub struct ChannelPeakAccumulator {
    peaks: Vec<f32>,
}

impl ChannelPeakAccumulator {
    pub fn new(channels: usize) -> Self {
        ChannelPeakAccumulator {
            peaks: vec![0.0; channels],
        }
    }

    /// Folds in one interleaved frame (`frame.len() == channels`).
    pub fn push_frame(&mut self, frame: &[f32]) {
        for (p, &s) in self.peaks.iter_mut().zip(frame.iter()) {
            let a = s.abs();
            if a > *p {
                *p = a;
            }
        }
    }

    pub fn finish(self) -> Vec<ChannelPeak> {
        self.peaks
            .into_iter()
            .map(|p| ChannelPeak {
                peak_dbfs: peak_to_dbfs(p),
            })
            .collect()
    }
}

/// SPEC-005 §2.4's silent-channel classification: exactly one channel peaks above
/// [`ACTIVE_CHANNEL_DBFS`] and every other channel is at or below [`SILENT_CHANNEL_DBFS`]. Returns
/// the active channel's index.
pub fn suggest_silent_channel(peaks: &[ChannelPeak]) -> Option<usize> {
    let mut active: Option<usize> = None;
    for (i, p) in peaks.iter().enumerate() {
        if p.peak_dbfs > ACTIVE_CHANNEL_DBFS {
            if active.is_some() {
                return None; // more than one active channel
            }
            active = Some(i);
        } else if p.peak_dbfs > SILENT_CHANNEL_DBFS {
            return None; // a channel that's neither clearly silent nor clearly active
        }
    }
    active
}

/// WAVE_FORMAT_EXTENSIBLE / symphonia `Position` speaker bit positions, in mask order (SPEC-005
/// §2.4's example labels; matches the first 18 WAVE channel mask bits).
const CHANNEL_NAMES: &[&str] = &[
    "Left",
    "Right",
    "Center",
    "LFE",
    "Surround Left",
    "Surround Right",
    "Left of Center",
    "Right of Center",
    "Back Center",
    "Side Left",
    "Side Right",
    "Top Center",
    "Top Front Left",
    "Top Front Center",
    "Top Front Right",
    "Top Back Left",
    "Top Back Center",
    "Top Back Right",
];
/// Bit index of the LFE position within the standard WAVE channel mask (`SPEAKER_LOW_FREQUENCY`).
const LFE_BIT: u32 = 3;

/// Builds [`ChannelInfo`] for every channel from an optional WAVE-style channel mask (SPEC-005
/// §2.4). Without a mask: 1 channel needs no label (mono, no downmix choice); 2 channels are
/// Left/Right (the common no-mask stereo case); otherwise every channel is "Channel N".
pub fn channel_infos(channel_count: u16, mask: Option<u32>) -> Vec<ChannelInfo> {
    if let Some(mask) = mask {
        let mut infos = Vec::with_capacity(channel_count as usize);
        let mut bit = 0u32;
        while infos.len() < channel_count as usize && bit < 32 {
            if mask & (1 << bit) != 0 {
                let label = CHANNEL_NAMES
                    .get(bit as usize)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("Channel {}", infos.len() + 1));
                infos.push(ChannelInfo {
                    label,
                    is_lfe: bit == LFE_BIT,
                });
            }
            bit += 1;
        }
        // The mask named fewer positions than there are channels (a malformed/partial mask):
        // pad with generic labels rather than panicking or dropping channels.
        while infos.len() < channel_count as usize {
            infos.push(ChannelInfo {
                label: format!("Channel {}", infos.len() + 1),
                is_lfe: false,
            });
        }
        infos
    } else if channel_count == 2 {
        vec![
            ChannelInfo {
                label: "Left".into(),
                is_lfe: false,
            },
            ChannelInfo {
                label: "Right".into(),
                is_lfe: false,
            },
        ]
    } else {
        (0..channel_count)
            .map(|i| ChannelInfo {
                label: format!("Channel {}", i + 1),
                is_lfe: false,
            })
            .collect()
    }
}

/// `true` if every channel is bit-identical over the probe window (SPEC-005 §2.4 "Identical
/// channels"): folds `push_frame` calls comparing each non-first channel against the first.
#[derive(Debug, Clone)]
pub struct IdenticalChannelsCheck {
    channels: usize,
    identical: Vec<bool>,
    any_frame: bool,
}

impl IdenticalChannelsCheck {
    pub fn new(channels: usize) -> Self {
        IdenticalChannelsCheck {
            channels,
            identical: vec![true; channels.max(1)],
            any_frame: false,
        }
    }

    #[allow(clippy::float_cmp)] // SPEC-005 §2.4 "identical channels" means bit-exact, not close
    pub fn push_frame(&mut self, frame: &[f32]) {
        self.any_frame = true;
        let first = frame.first().copied().unwrap_or(0.0);
        for (i, &s) in frame.iter().enumerate().skip(1) {
            if s != first {
                self.identical[i] = false;
            }
        }
    }

    /// `true` when there's more than one channel and every one of them equals the first,
    /// bit-exactly, over every frame seen.
    pub fn all_identical(&self) -> bool {
        self.channels > 1 && self.any_frame && self.identical.iter().skip(1).all(|&b| b)
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // deliberate bit-exact checks throughout (AC-9's whole point)
mod tests {
    use super::*;

    #[test]
    fn average_of_two_channels_uses_the_exact_half_factor() {
        let lfe = [false, false];
        assert_eq!(downmix_average(&[1.0, -1.0], &lfe), 0.0);
        assert_eq!(downmix_average(&[0.5, 0.5], &lfe), 0.5);
    }

    #[test]
    fn average_excludes_lfe_and_matches_the_reciprocal_multiply() {
        // 5.1 mask 0x3F order: FL, FR, FC, LFE, BL, BR (SPEC-005 AC-9e).
        let frame = [0.1f32, 0.2, 0.3, 0.9, 0.4, 0.5];
        let lfe = [false, false, false, true, false, false];
        let got = downmix_average(&frame, &lfe);
        let expected = (0.1f32 + 0.2 + 0.3 + 0.4 + 0.5) * (1.0f32 / 5.0f32);
        assert_eq!(got, expected);
    }

    #[test]
    fn average_of_identical_channels_is_bit_exact_to_either() {
        let lfe = [false, false];
        for x in [0.0f32, 0.25, -0.9999, 1.0] {
            assert_eq!(downmix_average(&[x, x], &lfe), x);
        }
    }

    #[test]
    fn pick_channel_copies_verbatim() {
        assert_eq!(downmix_pick(&[1.0, 2.0, 3.0], 1), 2.0);
        assert_eq!(downmix_pick(&[1.0, 2.0], 5), 0.0);
    }

    #[test]
    fn silent_channel_is_suggested_when_exactly_one_is_active() {
        let peaks = [
            ChannelPeak { peak_dbfs: -20.0 }, // active
            ChannelPeak {
                peak_dbfs: f32::NEG_INFINITY,
            }, // silent
        ];
        assert_eq!(suggest_silent_channel(&peaks), Some(0));
    }

    #[test]
    fn no_suggestion_when_both_channels_are_active() {
        let peaks = [
            ChannelPeak { peak_dbfs: -40.0 },
            ChannelPeak { peak_dbfs: -40.0 },
        ];
        assert_eq!(suggest_silent_channel(&peaks), None);
    }

    #[test]
    fn no_suggestion_in_the_ambiguous_middle_band() {
        // Right at -60 dBFS: neither <= -70 (silent) nor > -50 (active).
        let peaks = [
            ChannelPeak { peak_dbfs: -20.0 },
            ChannelPeak { peak_dbfs: -60.0 },
        ];
        assert_eq!(suggest_silent_channel(&peaks), None);
    }

    #[test]
    fn channel_infos_from_a_5_1_mask_matches_spec_order_and_lfe() {
        let infos = channel_infos(6, Some(0x3F));
        let labels: Vec<&str> = infos.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Left",
                "Right",
                "Center",
                "LFE",
                "Surround Left",
                "Surround Right"
            ]
        );
        assert!(infos[3].is_lfe);
        assert!(!infos[0].is_lfe);
    }

    #[test]
    fn channel_infos_without_a_mask_falls_back_to_left_right_or_channel_n() {
        let stereo = channel_infos(2, None);
        assert_eq!(stereo[0].label, "Left");
        assert_eq!(stereo[1].label, "Right");

        let quad = channel_infos(4, None);
        assert_eq!(quad[2].label, "Channel 3");
    }

    #[test]
    fn identical_channels_check_detects_equal_and_diverging_streams() {
        let mut same = IdenticalChannelsCheck::new(2);
        same.push_frame(&[0.5, 0.5]);
        same.push_frame(&[-0.25, -0.25]);
        assert!(same.all_identical());

        let mut diff = IdenticalChannelsCheck::new(2);
        diff.push_frame(&[0.5, 0.5]);
        diff.push_frame(&[-0.25, 0.25]);
        assert!(!diff.all_identical());

        // Mono has no "identical channels" concept.
        let mut mono = IdenticalChannelsCheck::new(1);
        mono.push_frame(&[0.5]);
        assert!(!mono.all_identical());
    }
}
