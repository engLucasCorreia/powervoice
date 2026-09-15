//! JSFX sliders → the module API's parameter schema (ADR-008 Amendment 11 §3).
//!
//! - **Identity:** `ParamId` = the slider's index (`slider1` → 0); `key` = `slider<N>`, JSFX's own
//!   numbering — the stable identifier REAPER and its presets use.
//! - **Enums** (`{a,b,c}`, and file sliders) → enum labels (values are indices).
//! - **Integer sliders** (an integral increment ≥ 1 and integral bounds) → stepped; `<0,1,1>` →
//!   `BOOL`. Other increments only set the shown decimals: JSFX sliders stay continuous for
//!   automation.
//! - **`:log` sliders** with a positive minimum → `Log` taper.
//! - **Hidden** (`-sliderN`) → `HIDDEN`; reversed ranges are ordered; degenerate ranges → hidden
//!   read-only 0..1 parameters (like CLAP's).
//! - No units: JSFX puts them in the name ("Gain (dB)").

use vox_module_api::{LocalizedText, ParamFlags, ParamId, ParamInfo, Taper, Unit};

/// One slider as the header declares it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RawSlider {
    pub(crate) index: u32,
    pub(crate) name: String,
    pub(crate) def: f64,
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) inc: f64,
    pub(crate) log: bool,
    pub(crate) enum_names: Vec<String>,
    pub(crate) visible: bool,
}

/// The parameter key of slider `index` (0-based).
pub(crate) fn key(index: u32) -> String {
    format!("slider{}", index + 1)
}

/// Decimals that show values in steps of `inc` (2 for continuous sliders).
pub(crate) fn decimals(inc: f64) -> u8 {
    if !(inc.is_finite() && inc > 0.0) {
        return 2;
    }
    (0u8..=6)
        .find(|&d| {
            let s = inc * 10f64.powi(i32::from(d));
            s >= 0.5 && (s - s.round()).abs() < 1e-6
        })
        .unwrap_or(6)
}

fn integral(v: f64) -> bool {
    v.is_finite() && v.fract() == 0.0
}

/// The schema of `sliders`, in header order.
pub(crate) fn map(sliders: &[RawSlider]) -> Vec<ParamInfo> {
    sliders
        .iter()
        .map(|r| {
            let name = if r.name.trim().is_empty() {
                format!("Slider {}", r.index + 1)
            } else {
                r.name.trim().to_owned()
            };
            let mut flags = if r.visible {
                ParamFlags::NONE
            } else {
                ParamFlags::HIDDEN
            };
            let mut info = ParamInfo {
                id: ParamId(r.index),
                key: key(r.index),
                name: LocalizedText::plain(&name),
                group: None,
                unit: Unit::None,
                min: 0.0,
                max: 1.0,
                default: 0.0,
                taper: Taper::Linear,
                step: None,
                enum_labels: Vec::new(),
                decimals: 0,
                smoothing_ms: 0.0,
                flags,
            };
            if r.enum_names.len() >= 2 {
                let max = (r.enum_names.len() - 1) as f64;
                info.max = max;
                info.default = if r.def.is_finite() {
                    r.def.round().clamp(0.0, max)
                } else {
                    0.0
                };
                info.step = Some(1.0);
                info.enum_labels = r
                    .enum_names
                    .iter()
                    .map(String::as_str)
                    .map(LocalizedText::plain)
                    .collect();
                info.flags = flags | ParamFlags::AUTOMATABLE | ParamFlags::STEPPED;
                return info;
            }
            let (mut lo, mut hi) = (r.min.min(r.max), r.min.max(r.max));
            let degenerate = !(lo.is_finite() && hi.is_finite()) || lo >= hi;
            if degenerate {
                lo = if lo.is_finite() { lo } else { 0.0 };
                hi = lo + 1.0;
                flags |= ParamFlags::READ_ONLY | ParamFlags::HIDDEN;
            } else {
                flags |= ParamFlags::AUTOMATABLE;
            }
            let stepped = !degenerate
                && r.inc >= 1.0
                && integral(r.inc)
                && integral(lo)
                && integral(hi)
                && integral((hi - lo) / r.inc);
            if stepped {
                flags |= ParamFlags::STEPPED;
                info.step = Some(r.inc);
                if lo == 0.0 && hi.to_bits() == 1f64.to_bits() {
                    flags |= ParamFlags::BOOL;
                }
            }
            info.min = lo;
            info.max = hi;
            info.default = if r.def.is_finite() {
                r.def.clamp(lo, hi)
            } else {
                lo
            };
            info.taper = if r.log && lo > 0.0 && !degenerate {
                Taper::Log
            } else {
                Taper::Linear
            };
            info.decimals = if stepped { 0 } else { decimals(r.inc) };
            info.flags = flags;
            info
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slider(index: u32, min: f64, max: f64, inc: f64) -> RawSlider {
        RawSlider {
            index,
            name: format!("S{index}"),
            def: min,
            min,
            max,
            inc,
            visible: true,
            ..RawSlider::default()
        }
    }

    #[test]
    fn sliders_become_parameters() {
        let mut gain = slider(0, -60.0, 24.0, 0.1);
        gain.def = 0.0;
        gain.name = " Gain (dB) ".into();
        let mut mode = slider(1, 0.0, 2.0, 1.0);
        mode.enum_names = vec!["Clean".into(), "Warm".into(), "Hot".into()];
        mode.def = 1.0;
        let mut freq = slider(2, 20.0, 20_000.0, 1.0);
        freq.log = true;
        let mut toggle = slider(3, 0.0, 1.0, 1.0);
        toggle.visible = false;
        let reversed = slider(4, 10.0, 0.0, 0.25);
        let flat = slider(5, 3.0, 3.0, 0.0);
        let p = map(&[gain, mode, freq, toggle, reversed, flat]);
        assert_eq!(
            (p[0].id, p[0].key.as_str(), p[0].name.text.as_str()),
            (ParamId(0), "slider1", "Gain (dB)")
        );
        assert_eq!(
            (p[0].min, p[0].max, p[0].default, p[0].decimals),
            (-60.0, 24.0, 0.0, 1)
        );
        assert_eq!(p[0].flags, ParamFlags::AUTOMATABLE);
        assert_eq!(p[0].step, None);
        let labels: Vec<&str> = p[1].enum_labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(labels, ["Clean", "Warm", "Hot"]);
        assert_eq!((p[1].max, p[1].default, p[1].step), (2.0, 1.0, Some(1.0)));
        assert!(p[1].flags.contains(ParamFlags::STEPPED));
        assert_eq!((p[2].taper, p[2].step), (Taper::Log, Some(1.0)));
        assert!(p[3].flags.contains(ParamFlags::BOOL | ParamFlags::HIDDEN));
        assert_eq!(
            (p[4].min, p[4].max, p[4].decimals, p[4].step),
            (0.0, 10.0, 2, None)
        );
        assert!(
            p[5].flags
                .contains(ParamFlags::READ_ONLY | ParamFlags::HIDDEN)
        );
        assert_eq!((p[5].min, p[5].max), (3.0, 4.0));
        assert!(!p[5].flags.contains(ParamFlags::AUTOMATABLE));
    }

    #[test]
    fn decimals_follow_the_increment() {
        assert_eq!(
            [0.0, 1.0, 0.1, 0.01, 0.25, 0.001, 1e-9].map(decimals),
            [2, 0, 1, 2, 2, 3, 6]
        );
    }
}
