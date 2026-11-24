//! After the main sequence: Hurley, Pols & Tout (2000) §5.1–5.4 and §6.
//!
//! The fits assume winds strip the envelope, which these tracks do not model. Without that, every
//! star would grow its core to the Chandrasekhar mass on the AGB, so the AGB ends when the core
//! reaches the white dwarf mass an initial–final mass relation gives instead.

use super::coefficients::{BLUE_LOOP_EXPONENT, Coefficients, ZAHB_EXPONENT, lg, pow};
use super::{Evolved, State, teff};

/// The `exp15(M − M_HeF)` of eqs. 49, 53, 56, 57.
const EXP_RATE: f64 = 15.0;
/// M☉ L☉⁻¹ Myr⁻¹: hydrogen burning on the TPAGB (eq. 71 with log A_H = −4.84), and helium burning
/// (eq. 68).
const A_H: f64 = 1.44e-5;
const A_HE: f64 = 7.66e-5;
/// M☉: the Chandrasekhar mass (§6.2.1), the core mass at the base of the AGB above which the core
/// collapses (§6), and the largest CO core that leaves a neutron star (eq. 75 ff.).
pub const M_CH: f64 = 1.44;
const MC_BAGB_COLLAPSE: f64 = 2.25;
const MC_SN_BLACK_HOLE: f64 = 7.0;

/// The core-mass–luminosity relation `L = min(B Mc^q, D Mc^p)` for one mass (eqs. 37–38).
#[derive(Clone, Copy, Debug)]
struct Relation {
    p: f64,
    q: f64,
    d: f64,
    b: f64,
    mx: f64,
    lx: f64,
}

impl Relation {
    fn for_mass(c: &Coefficients, m: f64) -> Self {
        let f = ((m - c.m_hef) / (2.5 - c.m_hef)).clamp(0.0, 1.0);
        let p = 6.0 - f;
        let q = 3.0 - f;
        let d0 = 5.37 + 0.135 * c.zeta;
        let d_hi = f64::max(-1.0, f64::max(0.975 * d0 - 0.18 * 2.5, 0.5 * d0 - 0.06 * 2.5));
        let log_d = if m >= 2.5 { f64::max(-1.0, f64::max(0.975 * d0 - 0.18 * m, 0.5 * d0 - 0.06 * m)) } else { d0 + (d_hi - d0) * f };
        let d = pow(10.0, log_d);
        let b = f64::max(3e4, 500.0 + 1.75e4 * pow(m, 0.6));
        let mx = pow(b / d, 1.0 / (p - q));
        Self { p, q, d, b, mx, lx: d * pow(mx, p) }
    }

    fn luminosity(&self, mc: f64) -> f64 {
        if mc <= self.mx { self.d * pow(mc, self.p) } else { self.b * pow(mc, self.q) }
    }

    fn core(&self, l: f64) -> f64 {
        if l <= self.lx { pow(l / self.d, 1.0 / self.p) } else { pow(l / self.b, 1.0 / self.q) }
    }
}

/// Luminosity and core mass in time from `(t0, l0)`, burning at rate `a` (eqs. 34–43).
#[derive(Clone, Copy, Debug)]
struct Track {
    r: Relation,
    a: f64,
    t_inf1: f64,
    t_x: f64,
    t_inf2: f64,
}

impl Track {
    fn new(r: Relation, a: f64, t0: f64, l0: f64) -> Self {
        let (p, q) = (r.p, r.q);
        let t_inf1 = t0 + 1.0 / ((p - 1.0) * a * r.d) * pow(r.d / l0, (p - 1.0) / p);
        let t_x = t_inf1 - (t_inf1 - t0) * pow(l0 / r.lx, (p - 1.0) / p);
        let t_inf2 = t_x + 1.0 / ((q - 1.0) * a * r.b) * pow(r.b / r.lx, (q - 1.0) / q);
        Self { r, a, t_inf1, t_x, t_inf2 }
    }

    fn luminosity(&self, t: f64) -> f64 {
        let r = &self.r;
        if t <= self.t_x {
            r.d * pow((r.p - 1.0) * self.a * r.d * (self.t_inf1 - t).max(f64::MIN_POSITIVE), r.p / (1.0 - r.p))
        } else {
            r.b * pow((r.q - 1.0) * self.a * r.b * (self.t_inf2 - t).max(f64::MIN_POSITIVE), r.q / (1.0 - r.q))
        }
    }

    fn core(&self, t: f64) -> f64 {
        let r = &self.r;
        if t <= self.t_x {
            pow((r.p - 1.0) * self.a * r.d * (self.t_inf1 - t).max(f64::MIN_POSITIVE), 1.0 / (1.0 - r.p))
        } else {
            pow((r.q - 1.0) * self.a * r.b * (self.t_inf2 - t).max(f64::MIN_POSITIVE), 1.0 / (1.0 - r.q))
        }
    }

    /// When the luminosity reaches `l` (eq. 43).
    fn time_at_luminosity(&self, l: f64) -> f64 {
        let r = &self.r;
        if l <= r.lx {
            self.t_inf1 - 1.0 / ((r.p - 1.0) * self.a * r.d) * pow(r.d / l, (r.p - 1.0) / r.p)
        } else {
            self.t_inf2 - 1.0 / ((r.q - 1.0) * self.a * r.b) * pow(r.b / l, (r.q - 1.0) / r.q)
        }
    }

    fn time_at_core(&self, mc: f64) -> f64 {
        self.time_at_luminosity(self.r.luminosity(mc))
    }
}

/// What remains.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum End {
    WhiteDwarf { mass: f64 },
    NeutronStar { mass: f64 },
    BlackHole { mass: f64 },
}

/// The core mass of a white dwarf from its progenitor's initial mass, M☉: the piecewise relation
/// SSE's `ifflag` option uses for any Z these tracks accept, standing in for the envelope loss
/// they do not model.
fn white_dwarf_mass(m: f64) -> f64 {
    let mc = f64::max(0.54 + 0.042 * m, f64::min(0.36 + 0.104 * m, 0.58 + 0.061 * m));
    if m < 1.0 { 0.46 } else { mc.min(M_CH) }
}

/// A neutron star of `1.17 + 0.09 Mc` (eq. 92) below the black-hole core, and above it the
/// remnant of Belczynski et al. (2002)'s fallback, which SSE offers as an option: Hurley's own
/// relation would make every black hole 1.8 M☉. With no winds the whole star is there to fall
/// back.
fn collapse(mc_sn: f64, m: f64) -> End {
    if mc_sn <= MC_SN_BLACK_HOLE {
        return End::NeutronStar { mass: 1.17 + 0.09 * mc_sn };
    }
    let fe_ni = if mc_sn < 2.5 { 0.161767 * mc_sn + 1.067055 } else { 0.314154 * mc_sn + 0.686088 };
    let fallback = ((mc_sn - 5.0) / 2.6).clamp(0.0, 1.0);
    End::BlackHole { mass: fe_ni + fallback * (m - fe_ni) }
}

/// One star's whole life past the main sequence, for one mass and metallicity. Times in Myr.
pub struct Life<'a> {
    c: &'a Coefficients,
    m: f64,
    pub t_ms: f64,
    pub t_bgb: f64,
    l_tms: f64,
    r_tms: f64,
    l_bgb: f64,
    giant_branch: Option<Track>,
    t_hei: f64,
    l_hei: f64,
    r_hei: f64,
    mc_hei: f64,
    t_he: f64,
    l_bagb: f64,
    early_agb: Track,
    /// Second dredge-up, and the thermally pulsing AGB after it, where there is one.
    pulsing: Option<(f64, f64, Track)>,
    pub t_end: f64,
    pub end: End,
}

impl<'a> Life<'a> {
    pub fn new(c: &'a Coefficients, m: f64) -> Self {
        let rel = Relation::for_mass(c, m);
        let (t_ms, t_bgb) = (c.t_ms(m), c.t_bgb(m));
        let (l_tms, r_tms) = c.main_sequence(m, t_ms);
        let l_bgb = c.l_bgb(m);
        let l_hei = l_hei(c, m);
        let (giant_branch, t_hei) = if m < c.m_fgb {
            let track = Track::new(rel, a_h_giant(m), t_bgb, l_bgb);
            (Some(track), track.time_at_luminosity(l_hei).max(t_bgb))
        } else {
            (None, t_bgb)
        };
        let mc_bagb = mc_bagb(c, m);
        let mc_hei = mc_hei(c, m, l_hei, mc_bagb);
        let t_he = t_he(c, m, mc_hei);
        let l_bagb = l_bagb(c, m);
        let t_bagb = t_hei + t_he;
        let early_agb = Track::new(rel, A_HE, t_bagb, l_bagb);

        let (pulsing, t_end, end) = if mc_bagb > MC_BAGB_COLLAPSE {
            let mc_sn = f64::max(M_CH, 0.773 * mc_bagb - 0.35);
            (None, early_agb.time_at_core(mc_sn).max(t_bagb), collapse(mc_sn, m))
        } else {
            // Eq. 69: second dredge-up, then the TPAGB with third dredge-up (eq. 73).
            let mc_du = if mc_bagb > 0.8 { 0.44 * mc_bagb + 0.448 } else { mc_bagb };
            let l_du = rel.luminosity(mc_du);
            let t_du = early_agb.time_at_luminosity(l_du).max(t_bagb);
            let pulsing = Track::new(rel, A_H * A_HE / (A_H + A_HE), t_du, l_du);
            let lambda = f64::min(0.9, 0.3 + 0.001 * pow(m, 5.0));
            let mass = white_dwarf_mass(m).max(early_agb.core(t_bagb)).min(M_CH);
            let t_end = if mass <= mc_du {
                early_agb.time_at_core(mass)
            } else {
                pulsing.time_at_core(mc_du + (mass - mc_du) / (1.0 - lambda))
            };
            (Some((t_du, mc_du, pulsing)), t_end.max(t_bagb), End::WhiteDwarf { mass })
        };
        let r_hei = r_hei(c, m, l_hei);
        Self {
            c,
            m,
            t_ms,
            t_bgb,
            l_tms,
            r_tms,
            l_bgb,
            giant_branch,
            t_hei,
            l_hei,
            r_hei,
            mc_hei,
            t_he,
            l_bagb,
            early_agb,
            pulsing,
            t_end,
            end,
        }
    }

    #[cfg(test)]
    pub fn t_hei(&self) -> f64 {
        self.t_hei
    }

    /// The star at `t`, `t_ms ≤ t < t_end`.
    pub fn at(&self, t: f64) -> Evolved {
        let (state, l, r) = if t < self.t_bgb {
            let (l, r) = self.hertzsprung_gap(t);
            (State::HertzsprungGap, l, r)
        } else if t < self.t_hei {
            let l = self.giant_branch.map_or(self.l_bgb, |g| g.luminosity(t));
            (State::GiantBranch, l, r_gb(self.c, self.m, l))
        } else if t < self.t_hei + self.t_he {
            let (l, r) = self.core_helium_burning((t - self.t_hei) / self.t_he);
            (State::CoreHeliumBurning, l, r)
        } else {
            let l = match &self.pulsing {
                Some((t_du, _, pulsing)) if t >= *t_du => pulsing.luminosity(t),
                _ => self.early_agb.luminosity(t),
            };
            (State::AsymptoticGiantBranch, l, r_agb(self.c, self.m, l))
        };
        Evolved { state, mass_solar: self.m, luminosity_solar: l, teff_k: teff(l, r), radius_solar: r }
    }

    /// Eqs. 25–27, geometric in L and R from the end of the main sequence.
    fn hertzsprung_gap(&self, t: f64) -> (f64, f64) {
        let (l_e, r_e) = if self.m < self.c.m_fgb {
            (self.l_bgb, r_gb(self.c, self.m, self.l_bgb))
        } else {
            (self.l_hei, self.r_hei)
        };
        let tau = ((t - self.t_ms) / (self.t_bgb - self.t_ms).max(f64::MIN_POSITIVE)).clamp(0.0, 1.0);
        (self.l_tms * pow(l_e / self.l_tms, tau), self.r_tms * pow(r_e / self.r_tms, tau))
    }

    /// Eqs. 58–65: luminosity rises from where helium burning settles to the base of the AGB, and
    /// the radius dips into a blue loop between `τ_x` and `τ_y`.
    fn core_helium_burning(&self, tau: f64) -> (f64, f64) {
        let (c, m) = (self.c, self.m);
        let r_mhe = r_mhe(c, m, self.mc_hei);
        let (tau_x, tau_y, l_x, r_x) = if m < c.m_hef {
            let l = l_zahb(c, m, self.mc_hei);
            (0.0, 1.0, l, r_zahb(c, m, self.mc_hei, l))
        } else if m < c.m_fgb {
            let l = l_min_he(c, m, self.l_hei);
            (1.0 - blue_fraction(c, m), 1.0, l, r_gb(c, m, l))
        } else {
            (0.0, blue_fraction(c, m), self.l_hei, self.r_hei)
        };
        let xi = (r_mhe / r_x).clamp(0.4, 2.5);
        let lum = |tau: f64| {
            if tau >= tau_x {
                let lambda = pow(((tau - tau_x) / (1.0 - tau_x).max(f64::MIN_POSITIVE)).clamp(0.0, 1.0), xi);
                l_x * pow(self.l_bagb / l_x, lambda)
            } else {
                let lambda = pow((tau_x - tau) / tau_x, 3.0);
                l_x * pow(self.l_hei / l_x, lambda)
            }
        };
        let l = lum(tau);
        let r = if tau < tau_x {
            r_gb(c, m, l)
        } else if tau > tau_y {
            r_agb(c, m, l)
        } else {
            let r_min = r_mhe.min(r_x);
            let r_y = r_agb(c, m, lum(tau_y));
            let span = (tau_y - tau_x).max(f64::MIN_POSITIVE);
            let rho = libm::cbrt(libm::log(r_y / r_min).max(0.0)) * (tau - tau_x) / span
                - libm::cbrt(libm::log(r_x / r_min).max(0.0)) * (tau_y - tau) / span;
            r_min * libm::exp(rho.abs().powi(3))
        };
        (l, r)
    }
}

/// log A_H' (§5.2): the burning rate the giant branch is fitted with.
fn a_h_giant(m: f64) -> f64 {
    pow(10.0, f64::max(-4.8, f64::min(-5.7 + 0.8 * m, -4.1 + 0.14 * m)))
}

/// Eq. 49.
fn l_hei(c: &Coefficients, m: f64) -> f64 {
    let b = &c.b;
    let hi = |m: f64| (b[11] + b[12] * pow(m, 3.8)) / (b[13] + m * m);
    if m >= c.m_hef {
        return hi(m);
    }
    let at = hi(c.m_hef);
    let alpha = (b[9] * pow(c.m_hef, b[10]) - at) / at;
    b[9] * pow(m, b[10]) / (1.0 + alpha * libm::exp(EXP_RATE * (m - c.m_hef)))
}

/// Eq. 66.
fn mc_bagb(c: &Coefficients, m: f64) -> f64 {
    let b = &c.b;
    pow(b[36] * pow(m, b[37]) + b[38], 0.25)
}

/// Core mass at helium ignition: from the core-mass–luminosity relation below M_HeF, and eq. 44
/// with L_HeI(M_HeF) above it.
fn mc_hei(c: &Coefficients, m: f64, l_hei: f64, mc_bagb: f64) -> f64 {
    if m < c.m_hef {
        return Relation::for_mass(c, m).core(l_hei).min(mc_bagb);
    }
    const C1: f64 = 9.20925e-5;
    const C2: f64 = 5.402216;
    let at_hef = Relation::for_mass(c, c.m_hef).core(self::l_hei(c, c.m_hef));
    let k = pow(at_hef, 4.0) - C1 * pow(c.m_hef, C2);
    f64::min(0.95 * mc_bagb, pow((k + C1 * pow(m, C2)).max(0.0), 0.25))
}

/// Eq. 56.
fn l_bagb(c: &Coefficients, m: f64) -> f64 {
    let b = &c.b;
    let hi = |m: f64| (b[31] + b[32] * pow(m, b[33] + 1.8)) / (b[34] + pow(m, b[33]));
    if m >= c.m_hef {
        return hi(m);
    }
    let at = hi(c.m_hef);
    let alpha = (b[29] * pow(c.m_hef, b[30]) - at) / at;
    b[29] * pow(m, b[30]) / (1.0 + alpha * libm::exp(EXP_RATE * (m - c.m_hef)))
}

/// Eq. 46.
pub fn r_gb(c: &Coefficients, m: f64, l: f64) -> f64 {
    let b = &c.b;
    let a = f64::min(b[4] * pow(m, -b[5]), b[6] * pow(m, -b[7]));
    a * (pow(l, b[1]) + b[2] * pow(l, b[3]))
}

/// Eq. 74, joined linearly across `[M_HeF − 0.2, M_HeF]` as the SSE code does.
fn r_agb(c: &Coefficients, m: f64, l: f64) -> f64 {
    let b = &c.b;
    let m1 = c.m_hef - 0.2;
    let hi = |m: f64| f64::min(b[51] * pow(m, -b[52]), b[53] * pow(m, -b[54]));
    let lo = |m: f64| b[56] + b[57] * m;
    let (a, exponent) = if m >= c.m_hef {
        (hi(m), b[55] * b[3])
    } else if m <= m1 {
        (lo(m), b[3])
    } else {
        let f = (m - m1) / 0.2;
        (lo(m1) + f * (hi(c.m_hef) - lo(m1)), b[3] * (1.0 + f * (b[55] - 1.0)))
    };
    a * (pow(l, b[1]) + b[2] * pow(l, exponent))
}

/// Eq. 50.
fn r_hei(c: &Coefficients, m: f64, l_hei: f64) -> f64 {
    let gb = r_gb(c, m, l_hei);
    if m <= c.m_fgb {
        return gb;
    }
    let r_mhe = r_mhe(c, m, 0.0);
    if m >= c.m_fgb.max(12.0) {
        return r_mhe;
    }
    let mu = lg(m / 12.0) / lg(c.m_fgb / 12.0);
    r_mhe * pow(gb / r_mhe, mu)
}

/// The naked helium-star ZAMS, Z = 0.02 (eqs. 77–79).
fn l_zhe(m: f64) -> f64 {
    15262.0 * pow(m, 10.25) / (pow(m, 9.0) + 29.54 * pow(m, 7.5) + 31.18 * pow(m, 6.0) + 0.0469)
}

fn r_zhe(m: f64) -> f64 {
    0.2391 * pow(m, 4.6) / (pow(m, 4.0) + 0.162 * m * m * m + 0.0065)
}

fn t_he_ms(m: f64) -> f64 {
    (0.4129 + 18.81 * pow(m, 4.0) + 1.853 * pow(m, 6.0)) / pow(m, 6.5)
}

/// Eq. 52's μ.
fn envelope_fraction(c: &Coefficients, m: f64, mc: f64) -> f64 {
    ((m - mc) / (c.m_hef - mc)).clamp(1e-12, 1.0)
}

/// Eq. 51.
fn l_min_he(c: &Coefficients, m: f64, l_hei: f64) -> f64 {
    let b = &c.b;
    let k = b[17] / pow(c.m_fgb, 0.1) + (b[16] * b[17] - b[14]) / pow(c.m_fgb, b[15] + 0.1);
    l_hei * (b[14] + k * pow(m, b[15] + 0.1)) / (b[16] + pow(m, b[15]))
}

/// Eq. 53.
fn l_zahb(c: &Coefficients, m: f64, mc: f64) -> f64 {
    let b = &c.b;
    let mu = envelope_fraction(c, m, mc);
    let base = l_zhe(mc);
    let l_min_hef = l_min_he(c, c.m_hef, l_hei(c, c.m_hef));
    let alpha = (b[18] + base - l_min_hef) / (l_min_hef - base);
    base + (1.0 + b[20]) / (1.0 + b[20] * pow(mu, ZAHB_EXPONENT)) * b[18] * pow(mu, b[19])
        / (1.0 + alpha * libm::exp(EXP_RATE * (m - c.m_hef)))
}

/// Eq. 54.
fn r_zahb(c: &Coefficients, m: f64, mc: f64, l_zahb: f64) -> f64 {
    let b = &c.b;
    let mu = envelope_fraction(c, m, mc);
    let f = (1.0 + b[21]) * pow(mu, b[22]) / (1.0 + b[21] * pow(mu, b[23]));
    (1.0 - f) * r_zhe(mc) + f * r_gb(c, m, l_zahb)
}

/// Eq. 55: the smallest radius in the blue loop. `mc` is used only below M_HeF.
fn r_mhe(c: &Coefficients, m: f64, mc: f64) -> f64 {
    let b = &c.b;
    let hi = |m: f64| (b[24] * m + pow(b[25] * m, b[26]) * pow(m, b[28])) / (b[27] + pow(m, b[28]));
    if m >= c.m_hef {
        return hi(m);
    }
    let l = l_zahb(c, m, mc);
    let mc_hef = mc_hei(c, c.m_hef, l_hei(c, c.m_hef), mc_bagb(c, c.m_hef));
    let at_hef = r_gb(c, c.m_hef, l_zahb(c, c.m_hef, mc_hef));
    r_gb(c, m, l) * pow(hi(c.m_hef) / at_hef, envelope_fraction(c, m, mc))
}

/// Eq. 57.
fn t_he(c: &Coefficients, m: f64, mc: f64) -> f64 {
    let b = &c.b;
    let hi = |m: f64| c.t_bgb(m) * (b[41] * pow(m, b[42]) + b[43] * pow(m, 5.0)) / (b[44] + pow(m, 5.0));
    if m >= c.m_hef {
        return hi(m);
    }
    let alpha = (hi(c.m_hef) - b[39]) / b[39];
    let mu = envelope_fraction(c, m, mc);
    (b[39] + (t_he_ms(mc) - b[39]) * pow(1.0 - mu, b[40])) * (1.0 + alpha * libm::exp(EXP_RATE * (m - c.m_hef)))
}

/// Eq. 58: the fraction of helium burning spent in the blue loop, `0 ≤ τ_bl ≤ 1`.
fn blue_fraction(c: &Coefficients, m: f64) -> f64 {
    let b = &c.b;
    let r = c.m_hef / c.m_fgb;
    let f = if m <= c.m_fgb {
        let m1 = m / c.m_fgb;
        let m2 = (lg(m1) / lg(r)).max(1e-12);
        b[45] * pow(m1, BLUE_LOOP_EXPONENT) + (1.0 - b[45] * pow(r, BLUE_LOOP_EXPONENT)) * pow(m2, b[46])
    } else {
        let f_bl = |m: f64| pow(m, b[48]) * pow((1.0 - r_mhe(c, m, 0.0) / r_agb(c, m, l_hei(c, m))).max(1e-12), b[49]);
        (1.0 - b[47]) * f_bl(m) / f_bl(c.m_fgb)
    };
    f.clamp(0.0, 1.0)
}
