//! Numbers as a reader sees them: `1.234 × 10⁻⁵`, not `1.234e-5`. For display only; nothing
//! parses this back, so a field a player types into keeps the `e` form.

const SUPERSCRIPT_DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
const SUPERSCRIPT_MINUS: char = '⁻';

/// `value` in scientific notation with `decimals` digits after the mantissa's point. A zero
/// exponent leaves the power off, so `scientific(2.5, 1)` is `2.5`.
pub fn scientific(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    // `-0.0` would keep its sign.
    let value = if value == 0.0 { 0.0 } else { value };
    let written = format!("{value:.decimals$e}");
    let (mantissa, exponent) = written.split_once('e').expect("`e` formatting writes an exponent");
    let exponent: i32 = exponent.parse().expect("`e` formatting writes an integer exponent");
    if exponent == 0 {
        return mantissa.to_string();
    }
    format!("{mantissa} × 10{}", superscript(exponent))
}

/// `n` in superscript digits, for an exponent.
pub fn superscript(n: i32) -> String {
    let mut out = String::new();
    if n < 0 {
        out.push(SUPERSCRIPT_MINUS);
    }
    out.extend(n.unsigned_abs().to_string().bytes().map(|d| SUPERSCRIPT_DIGITS[(d - b'0') as usize]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponents_are_raised_and_signed() {
        assert_eq!(scientific(1.234e-5, 3), "1.234 × 10⁻⁵");
        assert_eq!(scientific(-6.02e23, 2), "-6.02 × 10²³");
        assert_eq!(scientific(9.996e9, 2), "1.00 × 10¹⁰");
        assert_eq!(superscript(-1907), "⁻¹⁹⁰⁷");
    }

    #[test]
    fn a_zero_exponent_has_no_power() {
        assert_eq!(scientific(2.5, 1), "2.5");
        assert_eq!(scientific(0.0, 2), "0.00");
        assert_eq!(scientific(-0.0, 2), "0.00");
    }

    #[test]
    fn non_finite_values_are_named() {
        assert_eq!(scientific(f64::INFINITY, 2), "inf");
        assert_eq!(scientific(f64::NAN, 2), "NaN");
    }
}
