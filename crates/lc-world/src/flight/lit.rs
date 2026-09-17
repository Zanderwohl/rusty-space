//! How much rapidity a crossing's drive has put in, for [`crate::cost`].

use super::Cruise;

impl Cruise {
    /// The rapidity the drive has put in by `now_s`: its proper acceleration times the ship
    /// seconds spent lit, summed without regard to direction. What the drive pays for; see
    /// [`crate::cost`].
    pub fn lit_rapidity_at(&self, now_s: f64) -> f64 {
        let ordered = now_s - self.start_s;
        if ordered <= 0.0 {
            return 0.0;
        }
        let inv_gamma0 = (1.0 - self.beta0.length_squared()).max(0.0).sqrt();
        let turned = ordered.min(self.turn_s) * inv_gamma0;
        let line_s = ordered - self.turn_s - self.match_s;
        let coasted = if line_s > self.boost_s && self.brake_s > self.boost_s {
            let c = line_s.min(self.brake_s) - self.boost_s;
            let inv_gamma = (1.0 - self.coast_beta * self.coast_beta).max(0.0).sqrt();
            (c * inv_gamma).min(self.coast_proper_s)
        } else {
            0.0
        };
        let lit = (self.at(now_s).proper_s - turned - coasted).max(0.0);
        self.alpha * lit
    }

    /// [`Cruise::lit_rapidity_at`] for the whole crossing.
    pub fn planned_rapidity(&self) -> f64 {
        self.lit_rapidity_at(self.start_s + self.duration_s())
    }
}
