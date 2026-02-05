use std::ops::Sub;
use serde::{Deserialize, Serialize};

/// Stored in Seconds
/// Accurate to 1/100th second at 1,427,104 years on either side of epoch
/// Epoch in this program is J2000
#[derive(Serialize, Deserialize, Clone, Copy, PartialOrd, PartialEq, Default)]
pub struct Instant(f64);

// The instant of the J2000 epoch in Julian Days
pub(crate) const J2000_JD: f64 = 2451545.0;

/// The number of seconds in a Julian Day
pub const JD_SECONDS_PER_JULIAN_DAY: f64 = 24.0 * 60.0 * 60.0;


impl Instant {
    pub const J2000: Self = Self(J2000_JD);

    #[inline(always)]
    pub fn from_julian_day(julian_day: f64) -> Self {
        let seconds_since_j2000 = (julian_day - J2000_JD) * JD_SECONDS_PER_JULIAN_DAY;
        Self(seconds_since_j2000)
    }

    #[inline(always)]
    pub fn from_seconds_since_j2000(seconds: f64) -> Self {
        Self(seconds)
    }

    #[inline(always)]
    pub fn to_julian_day(&self) -> f64 {
        (self.0 / JD_SECONDS_PER_JULIAN_DAY) + J2000_JD
    }

    #[inline(always)]
    pub fn to_j2000_seconds(&self) -> f64 {
        self.0
    }
}

impl Sub for Instant {
    type Output = TimeDelta;

    fn sub(self, rhs: Self) -> Self::Output {
        TimeDelta::from_seconds(self.0 - rhs.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct TimeDelta(f64);

impl TimeDelta {
    pub fn from_seconds(seconds: f64) -> Self {
        Self(seconds)
    }

    pub fn to_seconds(&self) -> f64 {
        self.0
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct TimeLength(f64, Includes);

impl TimeLength {
    pub fn period_from_julian_day(julian_day: f64) -> Self {
        Self(julian_day * JD_SECONDS_PER_JULIAN_DAY, Includes::Beginning)
    }

    pub fn from_jd(jd: f64, includes: Includes) -> Self {
        Self(jd * JD_SECONDS_PER_JULIAN_DAY, includes)
    }

    pub fn from_seconds(in_seconds: f64, includes: Includes) -> Self {
        Self(in_seconds, includes)
    }

    pub fn to_seconds(&self) -> f64 {
        self.0
    }

    pub fn to_julian_days(&self) -> f64 {
        self.0 / JD_SECONDS_PER_JULIAN_DAY
    }
}

pub struct Span(f64, f64, Includes);

#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum Includes {
    Beginning,
    End,
    Both,
}

impl Span {

}
