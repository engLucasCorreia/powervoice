//! Locale-neutral parameter text conversion (ADR-005 §3, "Text rules").
//!
//! `.` is the decimal separator and `−` (U+2212) is accepted as minus. Rust is the single source
//! of truth for formatting; the UI never runs this code itself.

use crate::param::{ParamFlags, ParamInfo, Taper, Unit};

/// i18n key for the text `"On"` of BOOL parameters.
pub const TEXT_KEY_ON: &str = "param.on";
/// i18n key for the text `"Off"` of BOOL parameters.
pub const TEXT_KEY_OFF: &str = "param.off";

/// Text rule family, selected by enum labels, the BOOL flag, then the unit.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind<'a> {
    Enum,
    Bool,
    /// dB family; the displayed suffix.
    Db(&'static str),
    Hz,
    Ms,
    Seconds,
    Samples,
    Percent,
    Ratio,
    /// None / Custom: number only; the custom label (if any) is accepted when parsing.
    Plain(Option<&'a str>),
}

fn kind(p: &ParamInfo) -> Kind<'_> {
    if p.is_enum() {
        return Kind::Enum;
    }
    if p.flags.contains(ParamFlags::BOOL) {
        return Kind::Bool;
    }
    match &p.unit {
        Unit::Db => Kind::Db("dB"),
        Unit::Dbfs => Kind::Db("dBFS"),
        Unit::Dbtp => Kind::Db("dBTP"),
        Unit::Lufs => Kind::Db("LUFS"),
        Unit::Hz => Kind::Hz,
        Unit::Ms => Kind::Ms,
        Unit::Seconds => Kind::Seconds,
        Unit::Samples => Kind::Samples,
        Unit::Percent => Kind::Percent,
        Unit::Ratio => Kind::Ratio,
        Unit::Custom(label) => Kind::Plain(Some(label.as_str())),
        Unit::None => Kind::Plain(None),
    }
}

/// Formats with `decimals` digits; never prints a negative zero (`-0.0` → `0.0`).
fn fmt_num(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    match s.strip_prefix('-') {
        Some(rest) if rest.bytes().all(|b| b == b'0' || b == b'.') => rest.to_owned(),
        _ => s,
    }
}

/// `v` rounded to `decimals` digits (for unit-switch thresholds such as Hz → kHz).
fn round_to(v: f64, decimals: u8) -> f64 {
    let scale = 10f64.powi(i32::from(decimals));
    (v * scale).round() / scale
}

/// Parses a plain decimal number (optional sign incl. U+2212, digits, `.`, exponent).
/// Rejects `inf`/`nan` spellings and anything non-finite.
fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim().replace('\u{2212}', "-");
    if s.is_empty()
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-' | b'e' | b'E'))
    {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Case-insensitive ASCII suffix strip; the remainder is trimmed.
fn strip_suffix_ci<'s>(s: &'s str, suffix: &str) -> Option<&'s str> {
    let cut = s.len().checked_sub(suffix.len())?;
    if s.is_char_boundary(cut) && s[cut..].eq_ignore_ascii_case(suffix) {
        Some(s[..cut].trim_end())
    } else {
        None
    }
}

/// Strips the first matching suffix from `suffixes` (longest first) and returns the remainder
/// with the associated multiplier; no match returns the input with multiplier 1.
fn split_unit<'s>(s: &'s str, suffixes: &[(&str, f64)]) -> (&'s str, f64) {
    suffixes
        .iter()
        .find_map(|&(suf, mult)| strip_suffix_ci(s, suf).map(|rest| (rest, mult)))
        .unwrap_or((s, 1.0))
}

fn is_neg_inf(s: &str) -> bool {
    let s = s.trim().replace('\u{2212}', "-").to_lowercase();
    matches!(s.as_str(), "-inf" | "-∞" | "-infinity")
}

impl ParamInfo {
    /// Formats a plain value for display (the value is [`clamp_quantize`](Self::clamp_quantize)d
    /// first):
    ///
    /// | Kind | Display |
    /// |---|---|
    /// | Db / Dbfs / Dbtp / Lufs | `-6.0 dB`, `-1.0 dBTP`, `-inf dB` (Db taper with `neg_inf_at_min`, value `min`) |
    /// | Hz | `< 1000`: `250 Hz`; `≥ 1000`: `1.25 kHz` (2 decimals, more if the step needs them) |
    /// | Ms | `< 1000`: `12.0 ms`; `≥ 1000`: `1.20 s` (2 decimals, more if the step needs them) |
    /// | Seconds / Samples / Percent | `1.50 s`, `480 smp`, `50 %` |
    /// | Ratio | `4.0:1` |
    /// | Bool | `On` / `Off` (i18n keys [`TEXT_KEY_ON`] / [`TEXT_KEY_OFF`]) |
    /// | Enum | the label text |
    /// | None / Custom | the number |
    ///
    /// Numbers use [`decimals`](Self::decimals) (2 after a kHz / s switch). Stepped parameters
    /// get as many more decimals as their step (and `min`) need, so every legal value is shown
    /// exactly and round-trips exactly.
    pub fn value_to_text(&self, v: f64) -> String {
        let v = self.clamp_quantize(v);
        let (scale, decimals) = self.display_decimals(v);
        let d = usize::from(decimals);
        let switched = scale > 1.0;
        match kind(self) {
            Kind::Enum => {
                let idx = v.round().max(0.0) as usize;
                self.enum_labels
                    .get(idx)
                    .map_or_else(|| fmt_num(v, 0), |l| l.text.clone())
            }
            Kind::Bool => if v >= 0.5 { "On" } else { "Off" }.to_owned(),
            Kind::Db(suffix) => {
                if self.shows_neg_inf(v) {
                    format!("-inf {suffix}")
                } else {
                    format!("{} {suffix}", fmt_num(v, d))
                }
            }
            Kind::Hz if switched => format!("{} kHz", fmt_num(v / scale, d)),
            Kind::Hz => format!("{} Hz", fmt_num(v, d)),
            Kind::Ms if switched => format!("{} s", fmt_num(v / scale, d)),
            Kind::Ms => format!("{} ms", fmt_num(v, d)),
            Kind::Seconds => format!("{} s", fmt_num(v, d)),
            Kind::Samples => format!("{} smp", fmt_num(v, d)),
            Kind::Percent => format!("{} %", fmt_num(v, d)),
            Kind::Ratio => format!("{}:1", fmt_num(v, d)),
            Kind::Plain(_) => fmt_num(v, d),
        }
    }

    /// Parses user text into a plain value; `None` on garbage. The result is
    /// [`clamp_quantize`](Self::clamp_quantize)d. Units are optional and case-insensitive:
    ///
    /// | Kind | Accepts |
    /// |---|---|
    /// | Db / Dbfs / Dbtp / Lufs | `-6`, `-6 dB`, `−6db`, `-1 dBTP`, `-inf`/`−∞` (if `neg_inf_at_min`) |
    /// | Hz | `1250`, `1250 Hz`, `1.25k`, `1.25 kHz` |
    /// | Ms | `12`, `12 ms`, `1.2 s` |
    /// | Seconds / Samples / Percent | number with or without `s` / `smp` / `%` |
    /// | Ratio | `4`, `4:1` |
    /// | Bool | on/off/true/false/1/0 |
    /// | Enum | label (case-insensitive, trimmed) or integer index |
    /// | None / Custom | number (a trailing custom unit label is tolerated) |
    pub fn text_to_value(&self, s: &str) -> Option<f64> {
        let s = s.trim();
        let v = match kind(self) {
            Kind::Enum => {
                let lower = s.to_lowercase();
                match self
                    .enum_labels
                    .iter()
                    .position(|l| l.text.trim().to_lowercase() == lower)
                {
                    Some(i) => i as f64,
                    None => s.replace('\u{2212}', "-").parse::<i64>().ok()? as f64,
                }
            }
            Kind::Bool => match s.to_lowercase().as_str() {
                "on" | "true" | "1" => 1.0,
                "off" | "false" | "0" => 0.0,
                _ => return None,
            },
            Kind::Db(_) => {
                let (num, _) = split_unit(
                    s,
                    &[("dbfs", 1.0), ("dbtp", 1.0), ("lufs", 1.0), ("db", 1.0)],
                );
                if is_neg_inf(num) {
                    return match self.taper {
                        Taper::Db {
                            neg_inf_at_min: true,
                        } => Some(self.min),
                        _ => None,
                    };
                }
                parse_number(num)?
            }
            Kind::Hz => {
                let (num, mult) = split_unit(s, &[("khz", 1000.0), ("hz", 1.0), ("k", 1000.0)]);
                parse_number(num)? * mult
            }
            Kind::Ms => {
                let (num, mult) = split_unit(s, &[("ms", 1.0), ("s", 1000.0)]);
                parse_number(num)? * mult
            }
            Kind::Seconds => parse_number(split_unit(s, &[("s", 1.0)]).0)?,
            Kind::Samples => parse_number(split_unit(s, &[("smp", 1.0)]).0)?,
            Kind::Percent => parse_number(split_unit(s, &[("%", 1.0)]).0)?,
            Kind::Ratio => match s.split_once(':') {
                Some((a, b)) => {
                    let (a, b) = (parse_number(a)?, parse_number(b)?);
                    if b <= 0.0 {
                        return None;
                    }
                    a / b
                }
                None => parse_number(s)?,
            },
            Kind::Plain(label) => {
                let num = label
                    .filter(|l| !l.is_empty())
                    .and_then(|l| strip_suffix_ci(s, l))
                    .unwrap_or(s);
                parse_number(num)?
            }
        };
        Some(self.clamp_quantize(v))
    }

    /// True if `v` (already clamped) is displayed as `-inf`.
    fn shows_neg_inf(&self, v: f64) -> bool {
        matches!(
            self.taper,
            Taper::Db {
                neg_inf_at_min: true
            }
        ) && v <= self.min
    }

    /// Resolution of the displayed text at `v`, in plain units: one unit of the last shown
    /// decimal. 0 means the text is exact (enum, bool, `-inf`). Used by the round-trip contract:
    /// `text_to_value(value_to_text(v))` is within half of this of `clamp_quantize(v)`.
    #[cfg_attr(not(feature = "test-util"), allow(dead_code))] // used by ModuleTestHost
    pub(crate) fn text_resolution(&self, v: f64) -> f64 {
        let v = self.clamp_quantize(v);
        match kind(self) {
            Kind::Enum | Kind::Bool => 0.0,
            Kind::Db(_) if self.shows_neg_inf(v) => 0.0,
            _ => {
                let (scale, decimals) = self.display_decimals(v);
                scale * 10f64.powi(-i32::from(decimals))
            }
        }
    }

    /// The single source of display precision, for `value_to_text`, the unit-switch threshold
    /// and `text_resolution`. Returns `(scale, decimals)` for `v` (already clamp_quantized):
    /// `scale` is 1, or 1000 after the Hz → kHz / ms → s switch (taken when the value shows as
    /// ≥ 1000 in the base unit); `decimals` is the declared `decimals` (2 after a switch), raised
    /// for stepped parameters to what their step and `min` need in the displayed unit.
    fn display_decimals(&self, v: f64) -> (f64, u8) {
        let base = self.decimals.max(self.step_decimals(1.0));
        if matches!(kind(self), Kind::Hz | Kind::Ms) && round_to(v, base) >= 1000.0 {
            (1000.0, 2u8.max(self.step_decimals(1000.0)))
        } else {
            (1.0, base)
        }
    }

    /// Decimals needed to show every legal value `min + k*step` exactly in a unit of `scale`
    /// plain units (capped at 6); 0 for continuous parameters.
    fn step_decimals(&self, scale: f64) -> u8 {
        let Some(step) = self.step else {
            return 0;
        };
        let needed = |x: f64| {
            (0..=6u8)
                .find(|&d| {
                    let y = x / scale * 10f64.powi(i32::from(d));
                    (y - y.round()).abs() <= 1e-6 * y.abs().max(1.0)
                })
                .unwrap_or(6)
        };
        needed(step).max(needed(self.min))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::LocalizedText;
    use crate::param::tests::param;

    fn stepped(mut p: ParamInfo, step: f64) -> ParamInfo {
        p.step = Some(step);
        p.flags |= ParamFlags::STEPPED;
        p
    }

    #[test]
    fn stepped_values_keep_their_step_after_the_unit_switch() {
        let mut hz = stepped(param("freq_hz", Unit::Hz, 20.0, 20_000.0, Taper::Log), 1.0);
        hz.decimals = 0;
        assert_eq!(hz.value_to_text(250.0), "250 Hz");
        assert_eq!(hz.value_to_text(1253.0), "1.253 kHz");
        assert_eq!(hz.text_to_value("1.253 kHz"), Some(1253.0));
        assert!((hz.text_resolution(1253.0) - 1.0).abs() < 1e-9);

        let mut ms = stepped(param("hold_ms", Unit::Ms, 0.0, 5000.0, Taper::Linear), 1.0);
        ms.decimals = 0;
        assert_eq!(ms.value_to_text(12.0), "12 ms");
        assert_eq!(ms.value_to_text(1234.0), "1.234 s");
        assert_eq!(ms.text_to_value("1.234 s"), Some(1234.0));

        // A coarse step keeps the declared 2 kHz decimals.
        let mut coarse = stepped(param("f", Unit::Hz, 0.0, 20_000.0, Taper::Linear), 10.0);
        coarse.decimals = 0;
        assert_eq!(coarse.value_to_text(1250.0), "1.25 kHz");

        // Decimals are raised to show the step, and the grid offset of `min`.
        let mut db = stepped(
            param(
                "trim_db",
                Unit::Db,
                -12.0,
                12.0,
                Taper::Db {
                    neg_inf_at_min: false,
                },
            ),
            0.25,
        );
        db.decimals = 0;
        assert_eq!(db.value_to_text(0.75), "0.75 dB");
        let mut off = stepped(param("x", Unit::None, 0.05, 1.0, Taper::Linear), 0.1);
        off.decimals = 0;
        assert_eq!(off.value_to_text(0.15), "0.15");
        assert_eq!(
            off.text_to_value("0.15").map(f64::to_bits),
            Some(off.clamp_quantize(0.15).to_bits())
        );
    }

    #[test]
    fn db_display_and_parse() {
        let p = param(
            "gain_db",
            Unit::Db,
            -60.0,
            24.0,
            Taper::Db {
                neg_inf_at_min: true,
            },
        );
        assert_eq!(p.value_to_text(-6.0), "-6.0 dB");
        assert_eq!(p.value_to_text(-60.0), "-inf dB");
        assert_eq!(p.value_to_text(-0.04), "0.0 dB");
        assert_eq!(p.text_to_value("-6"), Some(-6.0));
        assert_eq!(p.text_to_value("-6 dB"), Some(-6.0));
        assert_eq!(p.text_to_value("\u{2212}6db"), Some(-6.0));
        assert_eq!(p.text_to_value("-inf"), Some(-60.0));
        assert_eq!(p.text_to_value("\u{2212}\u{221e}"), Some(-60.0));
        assert_eq!(p.text_to_value("-inf dB"), Some(-60.0));
        assert_eq!(p.text_to_value("100"), Some(24.0), "clamped");
        assert_eq!(p.text_to_value("abc"), None);
        assert_eq!(p.text_to_value("inf"), None);
        assert_eq!(p.text_to_value("nan"), None);
        assert_eq!(p.text_to_value(""), None);

        let tp = param(
            "ceiling_dbtp",
            Unit::Dbtp,
            -12.0,
            0.0,
            Taper::Db {
                neg_inf_at_min: false,
            },
        );
        assert_eq!(tp.value_to_text(-1.0), "-1.0 dBTP");
        assert_eq!(tp.text_to_value("-1 dBTP"), Some(-1.0));
        assert_eq!(
            tp.text_to_value("-inf"),
            None,
            "no -inf without neg_inf_at_min"
        );
        let lufs = param("target_lufs", Unit::Lufs, -40.0, 0.0, Taper::Linear);
        assert_eq!(lufs.value_to_text(-23.0), "-23.0 LUFS");
        assert_eq!(lufs.text_to_value("-23 LUFS"), Some(-23.0));
    }

    #[test]
    fn hz_display_and_parse() {
        let mut p = param("freq_hz", Unit::Hz, 20.0, 20_000.0, Taper::Log);
        p.decimals = 0;
        assert_eq!(p.value_to_text(250.0), "250 Hz");
        assert_eq!(p.value_to_text(1250.0), "1.25 kHz");
        assert_eq!(p.value_to_text(999.7), "1.00 kHz");
        for s in ["1250", "1250 Hz", "1.25k", "1.25 kHz", "1.25KHZ"] {
            assert_eq!(p.text_to_value(s), Some(1250.0), "{s}");
        }
    }

    #[test]
    fn ms_seconds_samples_percent_ratio() {
        let ms = param("attack_ms", Unit::Ms, 0.1, 5000.0, Taper::Log);
        assert_eq!(ms.value_to_text(12.0), "12.0 ms");
        assert_eq!(ms.value_to_text(1200.0), "1.20 s");
        assert_eq!(ms.text_to_value("12"), Some(12.0));
        assert_eq!(ms.text_to_value("12 ms"), Some(12.0));
        assert_eq!(ms.text_to_value("1.2 s"), Some(1200.0));

        let mut s = param("hold_s", Unit::Seconds, 0.0, 10.0, Taper::Linear);
        s.decimals = 2;
        assert_eq!(s.value_to_text(1.5), "1.50 s");
        assert_eq!(s.text_to_value("1.5"), Some(1.5));
        assert_eq!(s.text_to_value("1.5 s"), Some(1.5));

        let mut smp = stepped(
            param("delay", Unit::Samples, 0.0, 4800.0, Taper::Linear),
            1.0,
        );
        smp.decimals = 0;
        assert_eq!(smp.value_to_text(480.0), "480 smp");
        assert_eq!(smp.text_to_value("480 smp"), Some(480.0));
        assert_eq!(smp.text_to_value("480.4"), Some(480.0));

        let mut pct = param("mix", Unit::Percent, 0.0, 100.0, Taper::Linear);
        pct.decimals = 0;
        assert_eq!(pct.value_to_text(50.0), "50 %");
        assert_eq!(pct.text_to_value("50%"), Some(50.0));
        assert_eq!(pct.text_to_value("50 %"), Some(50.0));

        let ratio = param("ratio", Unit::Ratio, 1.0, 20.0, Taper::Log);
        assert_eq!(ratio.value_to_text(4.0), "4.0:1");
        assert_eq!(ratio.text_to_value("4"), Some(4.0));
        assert_eq!(ratio.text_to_value("4:1"), Some(4.0));
        assert_eq!(ratio.text_to_value("4 : 1"), Some(4.0));
        assert_eq!(ratio.text_to_value("4:0"), None);
    }

    #[test]
    fn bool_and_enum() {
        let mut b = stepped(param("enabled", Unit::None, 0.0, 1.0, Taper::Linear), 1.0);
        b.flags |= ParamFlags::BOOL;
        assert_eq!(b.value_to_text(1.0), "On");
        assert_eq!(b.value_to_text(0.0), "Off");
        for (s, v) in [
            ("on", 1.0),
            ("TRUE", 1.0),
            ("1", 1.0),
            ("Off", 0.0),
            ("false", 0.0),
            ("0", 0.0),
        ] {
            assert_eq!(b.text_to_value(s), Some(v), "{s}");
        }
        assert_eq!(b.text_to_value("maybe"), None);

        let mut e = stepped(param("mode", Unit::None, 0.0, 2.0, Taper::Linear), 1.0);
        e.enum_labels = ["Peak", "RMS", "Hybrid"].map(LocalizedText::plain).to_vec();
        assert_eq!(e.value_to_text(1.0), "RMS");
        assert_eq!(e.text_to_value(" rms "), Some(1.0));
        assert_eq!(e.text_to_value("HYBRID"), Some(2.0));
        assert_eq!(e.text_to_value("0"), Some(0.0));
        assert_eq!(e.text_to_value("9"), Some(2.0), "index clamped");
        assert_eq!(e.text_to_value("1.5"), None, "index must be an integer");
        assert_eq!(e.text_to_value("loud"), None);
    }

    #[test]
    fn plain_and_custom() {
        let mut p = param("q", Unit::None, 0.1, 10.0, Taper::Log);
        p.decimals = 2;
        assert_eq!(p.value_to_text(0.707), "0.71");
        assert_eq!(p.text_to_value("0.71"), Some(0.71));
        let mut c = param("amt", Unit::Custom("st".into()), -12.0, 12.0, Taper::Linear);
        c.decimals = 0;
        assert_eq!(c.value_to_text(3.0), "3");
        assert_eq!(c.text_to_value("3 st"), Some(3.0));
        assert_eq!(c.text_to_value("3"), Some(3.0));
    }

    #[test]
    fn text_resolution_matches_display() {
        let mut hz = param("freq_hz", Unit::Hz, 20.0, 20_000.0, Taper::Log);
        hz.decimals = 0;
        assert!((hz.text_resolution(500.0) - 1.0).abs() < 1e-12);
        assert!((hz.text_resolution(5000.0) - 10.0).abs() < 1e-12);
        let db = param(
            "g",
            Unit::Db,
            -60.0,
            0.0,
            Taper::Db {
                neg_inf_at_min: true,
            },
        );
        assert!((db.text_resolution(-6.0) - 0.1).abs() < 1e-12);
        assert!(db.text_resolution(-60.0).abs() < f64::EPSILON);
    }
}
