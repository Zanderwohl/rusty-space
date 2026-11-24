//! The main sequence: Hurley, Pols & Tout (2000) eqs. 4–24, on Tout et al. (1996)'s ZAMS.

use super::coefficients::{C1, C2, C3, Coefficients, lg, pow};

impl Coefficients {
    pub fn l_zams(&self, m: f64) -> f64 {
        let c = &self.zams_l;
        (c[0] * pow(m, 5.5) + c[1] * pow(m, 11.0))
            / (c[2] + m * m * m + c[3] * pow(m, 5.0) + c[4] * pow(m, 7.0) + c[5] * pow(m, 8.0) + c[6] * pow(m, 9.5))
    }

    pub fn r_zams(&self, m: f64) -> f64 {
        let c = &self.zams_r;
        let mx = libm::sqrt(m);
        ((c[0] * m * m + c[1] * pow(m, 6.0)) * mx + c[2] * pow(m, 11.0) + (c[3] + c[4] * mx) * pow(m, 19.0))
            / (c[5] + c[6] * m * m + (c[7] * pow(m, 8.0) + pow(m, 18.0) + c[8] * pow(m, 19.0)) * mx)
    }

    /// Myr. Eq. 4.
    pub fn t_bgb(&self, m: f64) -> f64 {
        let a = &self.a;
        (a[1] + a[2] * pow(m, 4.0) + a[3] * pow(m, 5.5) + pow(m, 7.0)) / (a[4] * m * m + a[5] * pow(m, 7.0))
    }

    /// Myr. Eqs. 5–7.
    pub(super) fn t_hook(&self, m: f64) -> f64 {
        let a = &self.a;
        let mu = f64::max(0.5, 1.0 - 0.01 * f64::max(a[6] / pow(m, a[7]), a[8] + a[9] / pow(m, a[10])));
        mu * self.t_bgb(m)
    }

    /// Myr. Eq. 5.
    pub fn t_ms(&self, m: f64) -> f64 {
        let x = f64::max(0.95, f64::min(0.95 - 0.03 * (self.zeta + 0.30103), 0.99));
        f64::max(self.t_hook(m), x * self.t_bgb(m))
    }

    /// Eq. 8.
    pub(super) fn l_tms(&self, m: f64) -> f64 {
        let a = &self.a;
        (a[11] * m * m * m + a[12] * pow(m, 4.0) + a[13] * pow(m, a[16] + 1.8)) / (a[14] + a[15] * pow(m, 5.0) + pow(m, a[16]))
    }

    /// Eq. 10.
    pub(super) fn l_bgb(&self, m: f64) -> f64 {
        let a = &self.a;
        (a[27] * pow(m, a[31]) + a[28] * pow(m, C2)) / (a[29] + a[30] * pow(m, C3) + pow(m, a[32]))
    }

    /// Eq. 9, joined linearly across `[a17, a17 + 0.1]`.
    pub(super) fn r_tms(&self, m: f64) -> f64 {
        let a = &self.a;
        let low = |m: f64| f64::max(1.5 * self.r_zams(m), (a[18] + a[19] * pow(m, a[21])) / (a[20] + pow(m, a[22])));
        let high = |m: f64| (C1 * m * m * m + a[23] * pow(m, a[26]) + a[24] * pow(m, a[26] + 1.5)) / (a[25] + pow(m, 5.0));
        let m2 = a[17] + 0.1;
        if m <= a[17] {
            low(m)
        } else if m >= m2 {
            high(m)
        } else {
            low(a[17]) + (high(m2) - low(a[17])) * (m - a[17]) / 0.1
        }
    }

    /// Eq. 19.
    fn alpha_l(&self, m: f64) -> f64 {
        let a = &self.a;
        let hi = |m: f64| (a[45] + a[46] * pow(m, a[48])) / (pow(m, 0.4) + a[47] * pow(m, 1.9));
        if m >= 2.0 {
            hi(m)
        } else if m < 0.5 {
            a[49]
        } else if m < 0.7 {
            a[49] + 5.0 * (0.3 - a[49]) * (m - 0.5)
        } else if m < a[52] {
            0.3 + (a[50] - 0.3) * (m - 0.7) / (a[52] - 0.7)
        } else if m < a[53] {
            a[50] + (a[51] - a[50]) * (m - a[52]) / (a[53] - a[52])
        } else {
            a[51] + (hi(2.0) - a[51]) * (m - a[53]) / (2.0 - a[53])
        }
    }

    /// Eq. 20.
    fn beta_l(&self, m: f64) -> f64 {
        let a = &self.a;
        let b = f64::max(0.0, a[54] - a[55] * pow(m, a[56]));
        if m > a[57] && b > 0.0 {
            let at = f64::max(0.0, a[54] - a[55] * pow(a[57], a[56]));
            f64::max(0.0, at - 10.0 * (m - a[57]) * at)
        } else {
            b
        }
    }

    /// Eq. 18.
    fn eta(&self, m: f64) -> f64 {
        let low_z = if self.z > 0.0009 { 10.0 } else { 20.0 };
        let by_mass = if m <= 1.0 {
            10.0
        } else if m >= 1.1 {
            20.0
        } else {
            10.0 + 100.0 * (m - 1.0)
        };
        by_mass.min(low_z)
    }

    /// Eq. 16.
    fn delta_l(&self, m: f64) -> f64 {
        let a = &self.a;
        let hi = |m: f64| f64::min(a[34] / pow(m, a[35]), a[36] / pow(m, a[37]));
        if m <= self.m_hook {
            0.0
        } else if m >= a[33] {
            hi(m)
        } else {
            hi(a[33]) * pow((m - self.m_hook) / (a[33] - self.m_hook), 0.4)
        }
    }

    /// Eq. 17.
    fn delta_r(&self, m: f64) -> f64 {
        let a = &self.a;
        let hi = |m: f64| (a[38] + a[39] * pow(m, 3.5)) / (a[40] * m * m * m + pow(m, a[41])) - 1.0;
        if m <= self.m_hook {
            0.0
        } else if m <= a[42] {
            a[43] * libm::sqrt((m - self.m_hook) / (a[42] - self.m_hook))
        } else if m < 2.0 {
            a[43] + (hi(2.0) - a[43]) * pow((m - a[42]) / (2.0 - a[42]), a[44])
        } else {
            hi(m)
        }
    }

    /// Eq. 21. The divisor is `a59 + M^a61`, as in the SSE code; the paper's HTML drops the `+`.
    fn alpha_r(&self, m: f64) -> f64 {
        let a = &self.a;
        let mid = |m: f64| a[58] * pow(m, a[60]) / (a[59] + pow(m, a[61]));
        if m < 0.5 {
            a[62]
        } else if m < 0.65 {
            a[62] + (a[63] - a[62]) * (m - 0.5) / 0.15
        } else if m < a[68] {
            a[63] + (a[64] - a[63]) * (m - 0.65) / (a[68] - 0.65)
        } else if m < a[66] {
            a[64] + (mid(a[66]) - a[64]) * (m - a[68]) / (a[66] - a[68])
        } else if m <= a[67] {
            mid(m)
        } else {
            mid(a[67]) + a[65] * (m - a[67])
        }
    }

    /// Eq. 22.
    fn beta_r(&self, m: f64) -> f64 {
        let a = &self.a;
        let mid = |m: f64| a[69] * pow(m, 3.5) / (a[70] + pow(m, a[71]));
        let b = if m <= 1.0 {
            1.06
        } else if m < a[74] {
            1.06 + (a[72] - 1.06) * (m - 1.0) / (a[74] - 1.06)
        } else if m < 2.0 {
            a[72] + (mid(2.0) - a[72]) * (m - a[74]) / (2.0 - a[74])
        } else if m <= 16.0 {
            mid(m)
        } else {
            mid(16.0) + a[73] * (m - 16.0)
        };
        b - 1.0
    }

    /// Eq. 23, never negative.
    fn gamma(&self, m: f64) -> f64 {
        let a = &self.a;
        if m > a[75] + 0.1 {
            return 0.0;
        }
        let b = f64::max(0.0, a[76] + a[77] * pow(1.0 - a[78], a[79]));
        let g = if m <= 1.0 {
            a[76] + a[77] * pow((m - a[78]).abs(), a[79])
        } else if m <= a[75] {
            b + (a[80] - b) * pow((m - 1.0) / (a[75] - 1.0), a[81])
        } else {
            let c = if a[75] > 1.0 { a[80] } else { b };
            c - 10.0 * (m - a[75]) * c
        };
        g.max(0.0)
    }

    /// `(L, R)` on the main sequence at `t` Myr, `t ≤ t_MS`. Eqs. 11–15.
    pub fn main_sequence(&self, m: f64, t: f64) -> (f64, f64) {
        let t_ms = self.t_ms(m);
        let t_hook = self.t_hook(m);
        let tau = (t / t_ms).clamp(0.0, 1.0);
        const EPS: f64 = 0.01;
        let tau1 = f64::min(1.0, t / t_hook);
        let tau2 = ((t - (1.0 - EPS) * t_hook) / (EPS * t_hook)).clamp(0.0, 1.0);
        let (lz, rz) = (self.l_zams(m), self.r_zams(m));

        let (al, bl, dl, eta) = (self.alpha_l(m), self.beta_l(m), self.delta_l(m), self.eta(m));
        let log_l = al * tau + bl * pow(tau, eta) + (lg(self.l_tms(m) / lz) - al - bl) * tau * tau
            - dl * (tau1 * tau1 - tau2 * tau2);

        let (ar, br, g, dr) = (self.alpha_r(m), self.beta_r(m), self.gamma(m), self.delta_r(m));
        let log_r = ar * tau + br * pow(tau, 10.0) + g * pow(tau, 40.0)
            + (lg(self.r_tms(m) / rz) - ar - br - g) * tau * tau * tau
            - dr * (tau1 * tau1 * tau1 - tau2 * tau2 * tau2);

        let mut r = rz * pow(10.0, log_r);
        if m < 0.1 {
            // Eq. 24, with X the hydrogen fraction.
            let x = 0.76 - 3.0 * self.z;
            r = r.max(0.0258 * pow(1.0 + x, 5.0 / 3.0) * pow(m, -1.0 / 3.0));
        }
        (lz * pow(10.0, log_l), r)
    }
}
