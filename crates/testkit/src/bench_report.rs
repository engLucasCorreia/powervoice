//! T-110: a tiny, machine-parseable convention for bench binaries (`benches/*.rs`, `harness =
//! false` or divan) to report one line per measured metric. `scripts/bench/summary.py` greps
//! `cargo bench --workspace`'s combined output for the `BENCH_RESULT` prefix and turns it into
//! `target/bench/summary.md`, each metric against its PROMPT/SPEC target where one exists
//! (T-110 ticket item 3).
//!
//! Not real-time code (bench setup/report time, never the audio thread): allocation is fine
//! here.

/// A target comparison for one reported metric. Omit ([`result`]'s `target: None`) for a metric
/// with no known PROMPT/SPEC number (still reported, just not judged pass/fail).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    /// The target value, in the metric's `unit`.
    pub value: f64,
    /// `Le`: the measured value must be at most `value`. `Ge`: at least `value`.
    pub op: TargetOp,
}

/// How a measured value compares to [`Target::value`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOp {
    /// Measured `<=` target (a budget: CPU %, latency, time).
    Le,
    /// Measured `>=` target (a floor: throughput).
    Ge,
}

impl Target {
    /// A budget target: pass iff the measured value is at most `value`.
    pub const fn le(value: f64) -> Self {
        Self {
            value,
            op: TargetOp::Le,
        }
    }

    /// A floor target: pass iff the measured value is at least `value`.
    pub const fn ge(value: f64) -> Self {
        Self {
            value,
            op: TargetOp::Ge,
        }
    }

    fn passes(self, value: f64) -> bool {
        match self.op {
            TargetOp::Le => value <= self.value,
            TargetOp::Ge => value >= self.value,
        }
    }

    fn op_str(self) -> &'static str {
        match self.op {
            TargetOp::Le => "le",
            TargetOp::Ge => "ge",
        }
    }
}

/// Prints one `BENCH_RESULT` line to stdout for `scripts/bench/summary.py`.
///
/// `crate_name`, `name` and `unit` must not contain whitespace (they become bare `key=value`
/// tokens); `name` should be stable across runs (it becomes a summary table row key).
pub fn result(crate_name: &str, name: &str, value: f64, unit: &str, target: Option<Target>) {
    debug_assert!(!crate_name.chars().any(char::is_whitespace));
    debug_assert!(!name.chars().any(char::is_whitespace));
    debug_assert!(!unit.chars().any(char::is_whitespace));
    match target {
        Some(t) => {
            let status = if t.passes(value) { "pass" } else { "fail" };
            println!(
                "BENCH_RESULT crate={crate_name} name={name} value={value} unit={unit} \
                 target={} op={} status={status}",
                t.value,
                t.op_str(),
            );
        }
        None => println!(
            "BENCH_RESULT crate={crate_name} name={name} value={value} unit={unit} target=- \
             op=- status=info"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_target_passes_at_or_under() {
        let t = Target::le(20.0);
        assert!(t.passes(20.0));
        assert!(t.passes(10.0));
        assert!(!t.passes(20.001));
    }

    #[test]
    fn ge_target_passes_at_or_over() {
        let t = Target::ge(48_000.0);
        assert!(t.passes(48_000.0));
        assert!(t.passes(96_000.0));
        assert!(!t.passes(47_999.9));
    }
}
