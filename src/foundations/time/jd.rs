
/// The J2000 epoch's Julian Day
pub const J2000_JD: f64 = 2451545.0;

/// The number of seconds in a Julian Day
pub const JD_SECONDS_PER_JULIAN_DAY: f64 = 24.0 * 60.0 * 60.0;

pub fn seconds_since_j2000(jd: f64) -> f64 {
    (jd - J2000_JD) * JD_SECONDS_PER_JULIAN_DAY
}
