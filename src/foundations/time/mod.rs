pub mod jd;

/// Stored in Seconds
pub struct Instant(f64);

impl From<f64> for Instant {
    fn from(val: f64) -> Self {
        Self(val)
    }
}
