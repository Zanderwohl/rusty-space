//! Instants, durations, and the epoch scales they convert through.
//!
//! Every type here is `#[repr(transparent)]` over `f64`, so it costs nothing at runtime
//! — after type checking these are just doubles.
//!
//! # The invariant that matters
//!
//! [`Instant`] counts **seconds since J2000**. [`JulianDate`] counts **days since
//! −4712-01-01 noon**. Those are different units *and* different origins, and mixing
//! them is exactly the bug this module is shaped to prevent: `Instant::J2000` once held
//! the Julian Day *number* 2451545.0, putting the J2000 epoch 28.37 days late and
//! shifting every body in the bundled solar system.
//!
//! So: no `From<f64>`, no `Deref`, no public fields, and no arithmetic between different
//! epoch types. A raw number only becomes a time through a named constructor, and epochs
//! only convert through [`From`].
//!
//! # Time scale
//!
//! [`Instant`] is nominally TT. TT−TDB stays under 2 ms, which is far below the accuracy
//! of anything here, so JPL ephemeris times in TDB are used directly.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::Sum;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// Seconds in a Julian day. Exact by definition — Julian days do not carry leap seconds.
pub const JD_SECONDS_PER_JULIAN_DAY: f64 = 24.0 * 60.0 * 60.0;

/// Days in a Julian year, by definition.
pub const DAYS_PER_JULIAN_YEAR: f64 = 365.25;

// ---------------------------------------------------------------------------
// Instant
// ---------------------------------------------------------------------------

/// A point in time, as **seconds since the J2000 epoch**.
///
/// Precise to 1/100 s out to roughly ±1.4 million years, which is where f64 spacing at
/// that magnitude reaches 0.01 s (see `docs/scratch/f64-time-precision-limits.py`).
#[repr(transparent)]
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct Instant(f64);

impl Instant {
    /// The J2000 epoch. `Instant` counts seconds *from* here, so this is zero.
    ///
    /// Its Julian Day number is [`JulianDate::J2000`]; the two are different units and
    /// must not be interchanged.
    pub const J2000: Self = Self(0.0);

    #[inline(always)]
    pub const fn from_seconds_since_j2000(seconds: f64) -> Self {
        Self(seconds)
    }

    #[inline(always)]
    pub const fn to_j2000_seconds(self) -> f64 {
        self.0
    }

    /// Convenience for `Instant::from(JulianDate::new(jd))`.
    #[inline]
    pub fn from_julian_day(julian_day: f64) -> Self {
        JulianDate::new(julian_day).into()
    }

    /// Convenience for `JulianDate::from(self).days()`.
    #[inline]
    pub fn to_julian_day(self) -> f64 {
        JulianDate::from(self).days()
    }

    #[inline]
    pub fn min(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }

    #[inline]
    pub fn max(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

// Total ordering via `total_cmp`, so `Instant` can key a sorted structure or a map.
// This orders NaN rather than poisoning comparisons; NaN is not an expected value.
//
// The `+ 0.0` normalises negative zero: IEEE totalOrder puts -0.0 strictly below +0.0,
// which would make two equal instants compare unequal and break the Ord/Eq/Hash
// agreement. Adding zero maps -0.0 to +0.0 and is the identity for every other value.
impl Ord for Instant {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        (self.0 + 0.0).total_cmp(&(other.0 + 0.0))
    }
}
impl PartialOrd for Instant {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Instant {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Instant {}

impl Hash for Instant {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Canonicalise -0.0 to 0.0 so equal values hash equally.
        let bits = if self.0 == 0.0 { 0f64.to_bits() } else { self.0.to_bits() };
        bits.hash(state);
    }
}

impl Sub for Instant {
    type Output = TimeDelta;
    #[inline]
    fn sub(self, rhs: Self) -> TimeDelta {
        TimeDelta(self.0 - rhs.0)
    }
}
impl Add<TimeDelta> for Instant {
    type Output = Instant;
    #[inline]
    fn add(self, rhs: TimeDelta) -> Instant {
        Instant(self.0 + rhs.0)
    }
}
impl Sub<TimeDelta> for Instant {
    type Output = Instant;
    #[inline]
    fn sub(self, rhs: TimeDelta) -> Instant {
        Instant(self.0 - rhs.0)
    }
}
impl AddAssign<TimeDelta> for Instant {
    #[inline]
    fn add_assign(&mut self, rhs: TimeDelta) {
        self.0 += rhs.0;
    }
}
impl SubAssign<TimeDelta> for Instant {
    #[inline]
    fn sub_assign(&mut self, rhs: TimeDelta) {
        self.0 -= rhs.0;
    }
}

impl fmt::Display for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "J2000{:+}s", self.0)
    }
}
impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Instant(J2000{:+}s = JD {})", self.0, self.to_julian_day())
    }
}

// ---------------------------------------------------------------------------
// TimeDelta
// ---------------------------------------------------------------------------

/// A signed duration in seconds.
///
/// Replaces the old `TimeLength`, which was this plus an `Includes` tag that nothing
/// ever read.
#[repr(transparent)]
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct TimeDelta(f64);

impl TimeDelta {
    pub const ZERO: Self = Self(0.0);

    #[inline(always)]
    pub const fn from_seconds(seconds: f64) -> Self {
        Self(seconds)
    }
    #[inline(always)]
    pub const fn to_seconds(self) -> f64 {
        self.0
    }
    #[inline]
    pub fn from_days(days: f64) -> Self {
        Self(days * JD_SECONDS_PER_JULIAN_DAY)
    }
    #[inline]
    pub fn to_days(self) -> f64 {
        self.0 / JD_SECONDS_PER_JULIAN_DAY
    }
    #[inline]
    pub fn from_julian_years(years: f64) -> Self {
        Self::from_days(years * DAYS_PER_JULIAN_YEAR)
    }
    #[inline]
    pub fn to_julian_years(self) -> f64 {
        self.to_days() / DAYS_PER_JULIAN_YEAR
    }
    #[inline]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }
    #[inline]
    pub fn is_zero(self) -> bool {
        self.0 == 0.0
    }
    #[inline]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

impl Ord for TimeDelta {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        // See the note on `impl Ord for Instant` for the `+ 0.0`.
        (self.0 + 0.0).total_cmp(&(other.0 + 0.0))
    }
}
impl PartialOrd for TimeDelta {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for TimeDelta {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for TimeDelta {}
impl Hash for TimeDelta {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let bits = if self.0 == 0.0 { 0f64.to_bits() } else { self.0.to_bits() };
        bits.hash(state);
    }
}

impl Add for TimeDelta {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
}
impl Sub for TimeDelta {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self { Self(self.0 - rhs.0) }
}
impl Neg for TimeDelta {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self { Self(-self.0) }
}
impl AddAssign for TimeDelta {
    #[inline]
    fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0; }
}
impl SubAssign for TimeDelta {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0; }
}
impl Mul<f64> for TimeDelta {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f64) -> Self { Self(self.0 * rhs) }
}
impl Mul<TimeDelta> for f64 {
    type Output = TimeDelta;
    #[inline]
    fn mul(self, rhs: TimeDelta) -> TimeDelta { TimeDelta(self * rhs.0) }
}
impl Div<f64> for TimeDelta {
    type Output = Self;
    #[inline]
    fn div(self, rhs: f64) -> Self { Self(self.0 / rhs) }
}
/// Dividing two durations gives a dimensionless ratio — how many of `rhs` fit in `self`.
impl Div for TimeDelta {
    type Output = f64;
    #[inline]
    fn div(self, rhs: Self) -> f64 { self.0 / rhs.0 }
}
impl Sum for TimeDelta {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |a, b| a + b)
    }
}

impl fmt::Display for TimeDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}s", self.0)
    }
}
impl fmt::Debug for TimeDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TimeDelta({}s = {} d)", self.0, self.to_days())
    }
}

// ---------------------------------------------------------------------------
// Epoch scales
// ---------------------------------------------------------------------------

/// A Julian Day number: days since −4712-01-01 12:00.
///
/// Days, not seconds, and a different origin from [`Instant`]. Convert with `From`.
#[repr(transparent)]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, PartialOrd, Default, Debug)]
pub struct JulianDate(f64);

impl JulianDate {
    /// The J2000 epoch as a Julian Day number: 2000-01-01 12:00 TT.
    pub const J2000: Self = Self(2451545.0);
    /// The B1950 (Besselian) epoch.
    pub const B1950: Self = Self(2433282.4235);
    /// JD of Modified Julian Date zero: 1858-11-17 00:00.
    pub const MJD_ZERO: Self = Self(2400000.5);
    /// JD of the Unix epoch: 1970-01-01 00:00.
    pub const UNIX_EPOCH: Self = Self(2440587.5);

    #[inline(always)]
    pub const fn new(days: f64) -> Self { Self(days) }
    #[inline(always)]
    pub const fn days(self) -> f64 { self.0 }
}

/// Modified Julian Date: `JD − 2400000.5`.
#[repr(transparent)]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, PartialOrd, Default, Debug)]
pub struct ModifiedJulianDate(f64);

impl ModifiedJulianDate {
    #[inline(always)]
    pub const fn new(days: f64) -> Self { Self(days) }
    #[inline(always)]
    pub const fn days(self) -> f64 { self.0 }
}

/// Seconds since the Unix epoch.
#[repr(transparent)]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, PartialOrd, Default, Debug)]
pub struct UnixTime(f64);

impl UnixTime {
    #[inline(always)]
    pub const fn new(seconds: f64) -> Self { Self(seconds) }
    #[inline(always)]
    pub const fn seconds(self) -> f64 { self.0 }
}

impl From<JulianDate> for Instant {
    #[inline]
    fn from(jd: JulianDate) -> Self {
        Instant((jd.0 - JulianDate::J2000.0) * JD_SECONDS_PER_JULIAN_DAY)
    }
}
impl From<Instant> for JulianDate {
    #[inline]
    fn from(t: Instant) -> Self {
        JulianDate(t.0 / JD_SECONDS_PER_JULIAN_DAY + JulianDate::J2000.0)
    }
}

impl From<ModifiedJulianDate> for JulianDate {
    #[inline]
    fn from(mjd: ModifiedJulianDate) -> Self {
        JulianDate(mjd.0 + JulianDate::MJD_ZERO.0)
    }
}
impl From<JulianDate> for ModifiedJulianDate {
    #[inline]
    fn from(jd: JulianDate) -> Self {
        ModifiedJulianDate(jd.0 - JulianDate::MJD_ZERO.0)
    }
}
impl From<ModifiedJulianDate> for Instant {
    #[inline]
    fn from(mjd: ModifiedJulianDate) -> Self {
        JulianDate::from(mjd).into()
    }
}
impl From<Instant> for ModifiedJulianDate {
    #[inline]
    fn from(t: Instant) -> Self {
        JulianDate::from(t).into()
    }
}

impl From<UnixTime> for Instant {
    #[inline]
    fn from(u: UnixTime) -> Self {
        Instant(u.0 - (JulianDate::J2000.0 - JulianDate::UNIX_EPOCH.0) * JD_SECONDS_PER_JULIAN_DAY)
    }
}
impl From<Instant> for UnixTime {
    #[inline]
    fn from(t: Instant) -> Self {
        UnixTime(t.0 + (JulianDate::J2000.0 - JulianDate::UNIX_EPOCH.0) * JD_SECONDS_PER_JULIAN_DAY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j2000_is_zero_seconds_and_2451545_days() {
        assert_eq!(Instant::J2000.to_j2000_seconds(), 0.0);
        assert_eq!(JulianDate::J2000.days(), 2451545.0);
        assert_eq!(Instant::from(JulianDate::J2000), Instant::J2000);
        assert_eq!(JulianDate::from(Instant::J2000), JulianDate::J2000);
    }

    #[test]
    fn epoch_scales_round_trip() {
        for jd in [2451545.0, 2451544.5, 2433282.4235, 2440587.5, 2400000.5, 2469807.5] {
            let j = JulianDate::new(jd);
            assert!((JulianDate::from(Instant::from(j)).days() - jd).abs() < 1e-6);
            assert!((JulianDate::from(ModifiedJulianDate::from(j)).days() - jd).abs() < 1e-6);
        }
        for secs in [0.0, 1.0e9, -1.0e9] {
            let t = Instant::from_seconds_since_j2000(secs);
            assert!((Instant::from(UnixTime::from(t)).to_j2000_seconds() - secs).abs() < 1e-3);
        }
    }

    #[test]
    fn unix_epoch_maps_to_the_right_instant() {
        // 1970-01-01 to 2000-01-01 12:00 is 10957.5 days.
        let t = Instant::from(UnixTime::new(0.0));
        assert!((t.to_j2000_seconds() + 10957.5 * 86400.0).abs() < 1e-6);
        assert_eq!(UnixTime::from(Instant::J2000).seconds(), 10957.5 * 86400.0);
    }

    #[test]
    fn time_algebra() {
        let t = Instant::from_julian_day(2451545.0);
        let d = TimeDelta::from_days(1.0);
        assert_eq!((t + d) - t, d);
        assert_eq!((t + d) - d, t);
        assert_eq!(d.to_seconds(), 86400.0);
        assert_eq!((-d).to_seconds(), -86400.0);
        assert_eq!((d * 2.0).to_days(), 2.0);
        assert_eq!(d / TimeDelta::from_days(0.5), 2.0);
        assert_eq!([d, d, d].into_iter().sum::<TimeDelta>().to_days(), 3.0);
        assert_eq!(TimeDelta::from_julian_years(1.0).to_days(), 365.25);

        let mut m = t;
        m += d;
        m -= d;
        assert_eq!(m, t);
    }

    #[test]
    fn instant_orders_and_hashes() {
        use std::collections::HashMap;
        let a = Instant::from_seconds_since_j2000(-0.0);
        let b = Instant::from_seconds_since_j2000(0.0);
        assert_eq!(a, b, "-0.0 and 0.0 must compare equal");

        let mut map = HashMap::new();
        map.insert(a, "first");
        map.insert(b, "second");
        assert_eq!(map.len(), 1, "-0.0 and 0.0 must hash to the same slot");

        let mut v = vec![
            Instant::from_seconds_since_j2000(5.0),
            Instant::from_seconds_since_j2000(-3.0),
            Instant::from_seconds_since_j2000(1.0),
        ];
        v.sort();
        assert_eq!(v[0].to_j2000_seconds(), -3.0);
        assert_eq!(v[2].to_j2000_seconds(), 5.0);
    }
}
