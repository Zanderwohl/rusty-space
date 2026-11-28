//! The world's rate as a shard is told it at boot: `--rate`, else `LC_RATE`, else the design rate.

use crate::server::DESIGN_RATE;

/// Coordinate seconds per real second: ten times the design rate.
///
/// The clock is `i64` microseconds, about 292,000 coordinate years: 33 real years from zero at
/// the design rate and 3.3 at this. At this rate retention keeps about 240 partitions per table.
pub const MAX_RATE: f64 = 10.0 * DESIGN_RATE as f64;

/// Coordinate seconds per real second.
///
/// `flag` is `Some(None)` for a `--rate` with nothing after it. An empty `LC_RATE` counts as
/// unset, because a compose file passes an unset variable through as empty.
pub fn stated(flag: Option<Option<&str>>, env: Option<&str>) -> Result<f64, String> {
    let (from, text) = match (flag, env.filter(|t| !t.trim().is_empty())) {
        (Some(None), _) => return Err("--rate needs a value".into()),
        (Some(Some(text)), _) => ("--rate", text),
        (None, Some(text)) => ("LC_RATE", text),
        (None, None) => return Ok(DESIGN_RATE as f64),
    };
    match text.trim().parse::<f64>() {
        Ok(rate) if rate.is_finite() && rate > 0.0 && rate <= MAX_RATE => Ok(rate),
        _ => Err(format!("{from}={text}: not a number of seconds above 0 and at most {MAX_RATE}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_wins_over_the_variable_and_the_variable_over_the_default() {
        assert_eq!(stated(Some(Some("100")), Some("526")), Ok(100.0));
        assert_eq!(stated(None, Some("526")), Ok(526.0));
        assert_eq!(stated(None, None), Ok(DESIGN_RATE as f64));
    }

    #[test]
    fn an_empty_variable_is_unset() {
        assert_eq!(stated(None, Some("")), Ok(DESIGN_RATE as f64));
        assert_eq!(stated(None, Some("  ")), Ok(DESIGN_RATE as f64));
    }

    #[test]
    fn a_rate_the_clock_cannot_hold_is_refused() {
        for bad in ["0", "-1", "abc", "inf", "NaN", "1e13"] {
            let refused = stated(Some(Some(bad)), None).unwrap_err();
            assert!(refused.starts_with("--rate="), "{refused}");
        }
        assert!(stated(None, Some("0")).unwrap_err().starts_with("LC_RATE="));
        assert_eq!(stated(Some(Some(&MAX_RATE.to_string())), None), Ok(MAX_RATE));
    }

    #[test]
    fn a_flag_with_no_value_is_refused_rather_than_ignored() {
        assert!(stated(Some(None), Some("526")).is_err());
    }

    #[test]
    fn the_clock_holds_years_at_the_ceiling() {
        let real_years = i64::MAX as f64 * 1e-6 / MAX_RATE / lc_world::flight::JULIAN_YEAR_S;
        assert!(real_years > 3.0, "{real_years}");
    }
}
