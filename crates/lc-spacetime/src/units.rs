//! Typed time.
//!
//! Coordinate time is stored once, as **`i64` microseconds from the world origin**, and
//! nothing else. There is no second absolute origin and no per-system epoch. `f64` seconds
//! exist only as a locally computed *difference*, which is what Kepler propagation consumes
//! and the regime where `f64` is precise. See `frame`.
//!
//! [`Micros`] is an instant and [`Span`] is a duration. They do not mix: two instants
//! subtract to a span, a span adds to an instant, and `Micros + Micros` does not compile.
//! This is the same discipline `em_foundations::time` applies to `Instant` and `JulianDate`,
//! and it exists for the same reason — mixing them once put a whole solar system 28 days out
//! of position.

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// Microseconds per second.
pub const MICROS_PER_SECOND: i64 = 1_000_000;

/// An instant in server-frame coordinate time: microseconds from the world origin.
#[repr(transparent)]
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Micros(i64);

impl Micros {
    /// The world origin.
    pub const ORIGIN: Self = Self(0);

    #[inline(always)]
    pub const fn new(micros: i64) -> Self {
        Self(micros)
    }

    #[inline(always)]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Seconds as `f64`. Lossy past a few decades from the origin; use only for display, or
    /// where the magnitude is known to be small. For propagation, take a difference first.
    #[inline]
    pub fn as_seconds_lossy(self) -> f64 {
        self.0 as f64 / MICROS_PER_SECOND as f64
    }
}

/// A duration in coordinate time, in microseconds. Signed.
#[repr(transparent)]
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span(i64);

impl Span {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn new(micros: i64) -> Self {
        Self(micros)
    }

    #[inline(always)]
    pub const fn get(self) -> i64 {
        self.0
    }

    #[inline]
    pub const fn from_seconds(seconds: i64) -> Self {
        Self(seconds * MICROS_PER_SECOND)
    }

    /// Seconds as `f64`. A span is a difference and is therefore small in the cases that
    /// matter, so this is the precise direction to convert in.
    #[inline]
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 / MICROS_PER_SECOND as f64
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }
}

impl Sub for Micros {
    type Output = Span;
    #[inline]
    fn sub(self, rhs: Self) -> Span {
        Span(self.0 - rhs.0)
    }
}
impl Add<Span> for Micros {
    type Output = Micros;
    #[inline]
    fn add(self, rhs: Span) -> Micros {
        Micros(self.0 + rhs.0)
    }
}
impl Sub<Span> for Micros {
    type Output = Micros;
    #[inline]
    fn sub(self, rhs: Span) -> Micros {
        Micros(self.0 - rhs.0)
    }
}
impl AddAssign<Span> for Micros {
    #[inline]
    fn add_assign(&mut self, rhs: Span) {
        self.0 += rhs.0;
    }
}
impl SubAssign<Span> for Micros {
    #[inline]
    fn sub_assign(&mut self, rhs: Span) {
        self.0 -= rhs.0;
    }
}
impl Add for Span {
    type Output = Span;
    #[inline]
    fn add(self, rhs: Self) -> Span {
        Span(self.0 + rhs.0)
    }
}
impl Sub for Span {
    type Output = Span;
    #[inline]
    fn sub(self, rhs: Self) -> Span {
        Span(self.0 - rhs.0)
    }
}
impl Neg for Span {
    type Output = Span;
    #[inline]
    fn neg(self) -> Span {
        Span(-self.0)
    }
}

impl std::fmt::Debug for Micros {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "t{:+}us", self.0)
    }
}
impl std::fmt::Debug for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:+}us", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_subtract_to_a_span_and_spans_add_to_instants() {
        let a = Micros::new(1_000);
        let b = Micros::new(1_750);
        assert_eq!((b - a).get(), 750);
        assert_eq!((a + Span::new(750)).get(), 1_750);
        assert_eq!((b - Span::new(750)).get(), 1_000);
    }

    #[test]
    fn a_span_converts_to_seconds_precisely() {
        assert_eq!(Span::from_seconds(3).as_seconds(), 3.0);
        assert_eq!(Span::new(1).as_seconds(), 1e-6);
    }
}
