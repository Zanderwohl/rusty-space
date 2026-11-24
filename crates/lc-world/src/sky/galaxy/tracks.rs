//! A star's state, luminosity and radius from its initial mass, age and metallicity.
//!
//! The zero-age main sequence is Tout et al. (1996, MNRAS 281, 257), and the main sequence and
//! Hertzsprung gap are Hurley, Pols & Tout (2000, MNRAS 315, 543, astro-ph/0001295), equations
//! and Appendix coefficients as published. Each table entry was checked against the SSE code's
//! data, where the paper's HTML and the code disagreed, on the form of α_R (eq. 21), the code is
//! taken. Valid for 0.1–100 M☉ and Z = 1e-4 to 0.03; outside that, masses and metallicities
//! are clamped to the edge.
//!
//! **The giant branch and later are still interim**: a fixed temperature and a luminosity rise
//! over a tenth of the main-sequence lifetime, until the paper's b_n fits replace them.
//!
//! Shared by the bake and the loader, so a published star is as bright in the game as in the
//! editor.

/// Solar effective temperature, K.
const T_SUN: f64 = 5772.0;

/// Hurley's ζ is `log10(Z / Z_REF)`.
const Z_REF: f64 = 0.02;
const Z_RANGE: (f64, f64) = (1e-4, 0.03);
const M_RANGE: (f64, f64) = (0.1, 100.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    MainSequence,
    /// The Hertzsprung gap and the giant branches.
    Giant,
    WhiteDwarf,
    NeutronStar,
    BlackHole,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Evolved {
    pub state: State,
    pub mass_solar: f64,
    pub luminosity_solar: f64,
    /// Zero for a remnant that emits nothing.
    pub teff_k: f64,
    pub radius_solar: f64,
}

/// Tout et al. (1996): `(a, b, c, d, e)` of a quartic in ζ for each ZAMS coefficient.
const ZAMS_L: [[f64; 5]; 7] = [
    [3.970417e-01, -3.2913574e-01, 3.4776688e-01, 3.7470851e-01, 9.011915e-02],
    [8.527626e+00, -2.441225973e+01, 5.643597107e+01, 3.706152575e+01, 5.4562406e+00],
    [2.5546e-04, -1.23461e-03, -2.3246e-04, 4.5519e-04, 1.6176e-04],
    [5.432889e+00, -8.62157806e+00, 1.344202049e+01, 1.451584135e+01, 3.39793084e+00],
    [5.563579e+00, -1.032345224e+01, 1.944322980e+01, 1.897361347e+01, 4.16903097e+00],
    [7.8866060e-01, -2.90870942e+00, 6.54713531e+00, 4.05606657e+00, 5.3287322e-01],
    [5.86685e-03, -1.704237e-02, 3.872348e-02, 2.570041e-02, 3.83376e-03],
];

/// θ, ι, κ, λ, μ, ν, ξ, ο, π. ν has no metallicity dependence.
const ZAMS_R: [[f64; 5]; 9] = [
    [1.715359e+00, 6.2246212e-01, -9.2557761e-01, -1.16996966e+00, -3.0631491e-01],
    [6.597788e+00, -4.2450044e-01, -1.213339427e+01, -1.073509484e+01, -2.51487077e+00],
    [1.008855000e+01, -7.11727086e+00, -3.167119479e+01, -2.424848322e+01, -5.33608972e+00],
    [1.012495e+00, 3.2699690e-01, -9.23418e-03, -3.876858e-02, -4.12750e-03],
    [7.490166e-02, 2.410413e-02, 7.233664e-02, 3.040467e-02, 1.97741e-03],
    [1.077422e-02, 0.0, 0.0, 0.0, 0.0],
    [3.082234e+00, 9.447205e-01, -2.15200882e+00, -2.49219496e+00, -6.3848738e-01],
    [1.784778e+01, -7.4534569e+00, -4.896066856e+01, -4.005386135e+01, -9.09331816e+00],
    [2.2582e-04, -1.86899e-03, 3.88783e-03, 1.42402e-03, -7.671e-05],
];

/// Hurley et al. (2000) Appendix: `(α, β, γ, η, μ)` for each tabulated a_n, indexed from 1. Rows
/// left zero are derived by formula in [`Coefficients::new`] or not needed before the giant
/// branch. a11, a12, a18, a19 and a29 are the primed values the formulas scale.
#[rustfmt::skip]
const A: [[f64; 5]; 82] = {
    let mut t = [[0.0; 5]; 82];
    t[1] = [1.593890e+3, 2.053038e+3, 1.231226e+3, 2.327785e+2, 0.0];
    t[2] = [2.706708e+3, 1.483131e+3, 5.772723e+2, 7.411230e+1, 0.0];
    t[3] = [1.466143e+2, -1.048442e+2, -6.795374e+1, -1.391127e+1, 0.0];
    t[4] = [4.141960e-2, 4.564888e-2, 2.958542e-2, 5.571483e-3, 0.0];
    t[5] = [3.426349e-1, 0.0, 0.0, 0.0, 0.0];
    t[6] = [1.949814e+1, 1.758178e+0, -6.008212e+0, -4.470533e+0, 0.0];
    t[7] = [4.903830e+0, 0.0, 0.0, 0.0, 0.0];
    t[8] = [5.212154e-2, 3.166411e-2, -2.750074e-3, -2.271549e-3, 0.0];
    t[9] = [1.312179e+0, -3.294936e-1, 9.231860e-2, 2.610989e-2, 0.0];
    t[10] = [8.073972e-1, 0.0, 0.0, 0.0, 0.0];
    t[11] = [1.031538e+0, -2.434480e-1, 7.732821e+0, 6.460705e+0, 1.374484e+0];
    t[12] = [1.043715e+0, -1.577474e+0, -5.168234e+0, -5.596506e+0, -1.299394e+0];
    t[13] = [7.859573e+2, -8.542048e+0, -2.642511e+1, -9.585707e+0, 0.0];
    t[14] = [3.858911e+3, 2.459681e+3, -7.630093e+1, -3.486057e+2, -4.861703e+1];
    t[15] = [2.888720e+2, 2.952979e+2, 1.850341e+2, 3.797254e+1, 0.0];
    t[16] = [7.196580e+0, 5.613746e-1, 3.805871e-1, 8.398728e-2, 0.0];
    t[18] = [2.187715e-1, -2.154437e+0, -3.768678e+0, -1.975518e+0, -3.021475e-1];
    t[19] = [1.466440e+0, 1.839725e+0, 6.442199e+0, 4.023635e+0, 6.957529e-1];
    t[20] = [2.652091e+1, 8.178458e+1, 1.156058e+2, 7.633811e+1, 1.950698e+1];
    t[21] = [1.472103e+0, -2.947609e+0, -3.312828e+0, -9.945065e-1, 0.0];
    t[22] = [3.071048e+0, -5.679941e+0, -9.745523e+0, -3.594543e+0, 0.0];
    t[23] = [2.617890e+0, 1.019135e+0, -3.292551e-2, -7.445123e-2, 0.0];
    t[24] = [1.075567e-2, 1.773287e-2, 9.610479e-3, 1.732469e-3, 0.0];
    t[25] = [1.476246e+0, 1.899331e+0, 1.195010e+0, 3.035051e-1, 0.0];
    t[26] = [5.502535e+0, -6.601663e-2, 9.968707e-2, 3.599801e-2, 0.0];
    t[27] = [9.511033e+1, 6.819618e+1, -1.045625e+1, -1.474939e+1, 0.0];
    t[28] = [3.113458e+1, 1.012033e+1, -4.650511e+0, -2.463185e+0, 0.0];
    t[29] = [1.413057e+0, 4.578814e-1, -6.850581e-2, -5.588658e-2, 0.0];
    t[30] = [3.910862e+1, 5.196646e+1, 2.264970e+1, 2.873680e+0, 0.0];
    t[31] = [4.597479e+0, -2.855179e-1, 2.709724e-1, 0.0, 0.0];
    t[32] = [6.682518e+0, 2.827718e-1, -7.294429e-2, 0.0, 0.0];
    t[34] = [1.910302e-1, 1.158624e-1, 3.348990e-2, 2.599706e-3, 0.0];
    t[35] = [3.931056e-1, 7.277637e-2, -1.366593e-1, -4.508946e-2, 0.0];
    t[36] = [3.267776e-1, 1.204424e-1, 9.988332e-2, 2.455361e-2, 0.0];
    t[37] = [5.990212e-1, 5.570264e-2, 6.207626e-2, 1.777283e-2, 0.0];
    t[38] = [7.330122e-1, 5.192827e-1, 2.316416e-1, 8.346941e-3, 0.0];
    t[39] = [1.172768e+0, -1.209262e-1, -1.193023e-1, -2.859837e-2, 0.0];
    t[40] = [3.982622e-1, -2.296279e-1, -2.262539e-1, -5.219837e-2, 0.0];
    t[41] = [3.571038e+0, -2.223625e-2, -2.611794e-2, -6.359648e-3, 0.0];
    t[45] = [2.321400e-1, 1.828075e-3, -2.232007e-2, -3.378734e-3, 0.0];
    t[46] = [1.163659e-2, 3.427682e-3, 1.421393e-3, -3.710666e-3, 0.0];
    t[47] = [1.048020e-2, -1.231921e-2, -1.686860e-2, -4.234354e-3, 0.0];
    t[48] = [1.555590e+0, -3.223927e-1, -5.197429e-1, -1.066441e-1, 0.0];
    t[54] = [3.855707e-1, -6.104166e-1, 5.676742e+0, 1.060894e+1, 5.284014e+0];
    t[55] = [3.579064e-1, -6.442936e-1, 5.494644e+0, 1.054952e+1, 5.280991e+0];
    t[56] = [9.587587e-1, 8.777464e-1, 2.017321e-1, 0.0, 0.0];
    t[58] = [4.907546e-1, -1.683928e-1, -3.108742e-1, -7.202918e-2, 0.0];
    t[59] = [4.537070e+0, -4.465455e+0, -1.612690e+0, -1.623246e+0, 0.0];
    t[60] = [1.796220e+0, 2.814020e-1, 1.423325e+0, 3.421036e-1, 0.0];
    t[61] = [2.256216e+0, 3.773400e-1, 1.537867e+0, 4.396373e-1, 0.0];
    t[65] = [1.564231e-3, 1.653042e-3, -4.439786e-3, -4.951011e-3, -1.216530e-3];
    t[67] = [5.210157e+0, -4.143695e+0, -2.120870e+0, 0.0, 0.0];
    t[69] = [1.071489e+0, -1.164852e-1, -8.623831e-2, -1.582349e-2, 0.0];
    t[70] = [7.108492e-1, 7.935927e-1, 3.926983e-1, 3.622146e-2, 0.0];
    t[71] = [3.478514e+0, -2.585474e-2, -1.512955e-2, -2.833691e-3, 0.0];
    t[72] = [9.132108e-1, -1.653695e-1, 0.0, 3.636784e-2, 0.0];
    t[73] = [3.969331e-3, 4.539076e-3, 1.720906e-3, 1.897857e-4, 0.0];
    t[76] = [1.192334e-2, 1.083057e-2, 1.230969e+0, 1.551656e+0, 0.0];
    t[77] = [-1.668868e-1, 5.818123e-1, -1.105027e+1, -1.668070e+1, 0.0];
    t[78] = [7.615495e-1, 1.068243e-1, -2.011333e-1, -9.371415e-2, 0.0];
    t[79] = [9.409838e+0, 1.522928e+0, 0.0, 0.0, 0.0];
    t
};

/// R_TMS's constant c1 and L_BGB's exponents c2, c3.
const C1: f64 = -8.672073e-2;
const C2: f64 = 9.301992;
const C3: f64 = 4.637345;

fn quartic(c: &[f64; 5], x: f64) -> f64 {
    c[0] + x * (c[1] + x * (c[2] + x * (c[3] + x * c[4])))
}

fn pow(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}

fn lg(x: f64) -> f64 {
    libm::log10(x)
}

/// Every metallicity-dependent number the formulas read, for one Z.
pub struct Coefficients {
    z: f64,
    zeta: f64,
    a: [f64; 82],
    zams_l: [f64; 7],
    zams_r: [f64; 9],
    m_hook: f64,
    m_fgb: f64,
}

impl Coefficients {
    pub fn new(z: f64) -> Self {
        let z = z.clamp(Z_RANGE.0, Z_RANGE.1);
        let zeta = lg(z / Z_REF);
        let mut a = [0.0; 82];
        for (n, row) in A.iter().enumerate() {
            a[n] = quartic(row, zeta);
        }
        a[11] *= a[14];
        a[12] *= a[14];
        a[18] *= a[20];
        a[19] *= a[20];
        a[29] = pow(a[29], a[32]);
        // The clamped and formula entries, as the Appendix gives them.
        let s = lg(z);
        a[17] = pow(10.0, f64::max(0.097 - 0.1072 * (s + 3.0), f64::max(0.097, f64::min(0.1461, 0.1461 + 0.1237 * (s + 2.0)))));
        let hook_mass = |zeta: f64| f64::max(0.6355 - 0.4192 * zeta, f64::max(1.25, f64::min(1.4, 1.5135 + 0.3769 * zeta)));
        a[33] = hook_mass(zeta);
        a[57] = hook_mass(zeta);
        a[42] = (1.9848 + zeta * (1.1386 + 0.3564 * zeta)).clamp(1.1, 1.25);
        a[43] = 0.063 + zeta * (0.0481 + 0.00984 * zeta);
        a[44] = (1.2 + 2.45 * zeta).clamp(0.45, 1.3);
        a[49] = f64::max(0.145, 0.0977 - zeta * (0.231 + 0.0753 * zeta));
        a[50] = f64::min(0.24 + zeta * (0.18 + 0.595 * zeta), 0.306 + 0.053 * zeta);
        a[51] = f64::min(0.33 + zeta * (0.132 + 0.218 * zeta), 0.3625 + 0.062 * zeta);
        a[52] = f64::max(0.9, 1.1064 + zeta * (0.415 + 0.18 * zeta));
        a[53] = f64::max(1.0, 1.19 + zeta * (0.377 + 0.176 * zeta));
        if z > 0.01 {
            a[52] = a[52].min(1.0);
            a[53] = a[53].min(1.1);
        }
        a[62] = f64::max(0.065, 0.0843 - zeta * (0.0475 + 0.0352 * zeta));
        a[63] = 0.0736 + zeta * (0.0749 + 0.04426 * zeta);
        if z < 0.004 {
            a[63] = a[63].min(0.055);
        }
        a[64] = (0.136 + 0.0352 * zeta).clamp(0.091, 0.121);
        a[66] = f64::max(1.477 + 0.296 * zeta, f64::min(1.6, -0.308 - 1.046 * zeta));
        a[66] = f64::max(0.8, f64::min(0.8 - 2.0 * zeta, a[66]));
        a[68] = (1.116 + 0.166 * zeta).clamp(0.9, 1.0);
        if a[68] > a[66] {
            a[68] = a[66];
            a[64] = a[58] * pow(a[66], a[60]) / (a[59] + pow(a[66], a[61]));
        }
        if z > 0.01 {
            a[72] = a[72].max(0.95);
        }
        a[74] = (1.6 + zeta * (0.764 + 0.3322 * zeta)).clamp(1.4, 1.6);
        a[75] = (0.8109 - 0.6282 * zeta).clamp(1.0, 1.27).max(0.6355 - 0.4192 * zeta);
        a[76] = a[76].max(-0.1015564 - 0.2161264 * zeta - 0.05182516 * zeta * zeta);
        a[77] = a[77].min(0.0).max(-0.3868776 - 0.5457078 * zeta - 0.1463472 * zeta * zeta);
        a[78] = a[78].min(7.454 + 9.046 * zeta).max(0.0);
        a[79] = a[79].min(f64::max(2.0, -13.3 - 18.6 * zeta));
        a[80] = f64::max(5.855420e-2, -0.2711 - zeta * (0.5756 + 0.0838 * zeta));
        a[81] = (2.493 + 1.1475 * zeta).clamp(0.4, 1.5);

        let mut zams_l = [0.0; 7];
        for (k, row) in ZAMS_L.iter().enumerate() {
            zams_l[k] = quartic(row, zeta);
        }
        let mut zams_r = [0.0; 9];
        for (k, row) in ZAMS_R.iter().enumerate() {
            zams_r[k] = quartic(row, zeta);
        }
        Self {
            z,
            zeta,
            a,
            zams_l,
            zams_r,
            m_hook: 1.0185 + zeta * (0.16015 + 0.0892 * zeta),
            m_fgb: 13.048 * pow(z / Z_REF, 0.06) / (1.0 + 0.0012 * pow(Z_REF / z, 1.27)),
        }
    }

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
    fn t_hook(&self, m: f64) -> f64 {
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
    fn l_tms(&self, m: f64) -> f64 {
        let a = &self.a;
        (a[11] * m * m * m + a[12] * pow(m, 4.0) + a[13] * pow(m, a[16] + 1.8)) / (a[14] + a[15] * pow(m, 5.0) + pow(m, a[16]))
    }

    /// Eq. 10.
    fn l_bgb(&self, m: f64) -> f64 {
        let a = &self.a;
        (a[27] * pow(m, a[31]) + a[28] * pow(m, C2)) / (a[29] + a[30] * pow(m, C3) + pow(m, a[32]))
    }

    /// Eq. 9, joined linearly across `[a17, a17 + 0.1]`.
    fn r_tms(&self, m: f64) -> f64 {
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

/// `[Fe/H]` to Hurley's Z, scaled from the solar Z = 0.02 the fits use.
pub fn metallicity_z(feh: f64) -> f64 {
    Z_REF * pow(10.0, feh)
}

/// The interim giant temperature, K, until the giant-branch radius fits replace it.
const GIANT_TEFF_K: f64 = 4500.0;
const SUPERGIANT_TEFF_K: f64 = 4000.0;
/// The interim giant phases last this fraction of the main-sequence lifetime past the base of
/// the giant branch, reaching this many times L_BGB at the tip.
const GIANT_SPAN: f64 = 0.1;
const GIANT_RISE: f64 = 30.0;

pub fn evolve(initial_mass: f64, age_gyr: f64, feh: f64) -> Evolved {
    let m = initial_mass.clamp(M_RANGE.0, M_RANGE.1);
    let c = Coefficients::new(metallicity_z(feh));
    let t = age_gyr.max(0.0) * 1000.0;
    let (t_ms, t_bgb) = (c.t_ms(m), c.t_bgb(m));
    let t_end = t_bgb * (1.0 + GIANT_SPAN);

    if t < t_ms {
        let (l, r) = c.main_sequence(m, t);
        return Evolved { state: State::MainSequence, mass_solar: initial_mass, luminosity_solar: l, teff_k: teff(l, r), radius_solar: r };
    }
    if t < t_end {
        let (l_tms, r_tms) = c.main_sequence(m, t_ms);
        let giant_t = if m < c.m_fgb { GIANT_TEFF_K } else { SUPERGIANT_TEFF_K };
        let l_bgb = if m < c.m_fgb { c.l_bgb(m) } else { l_tms.max(c.l_bgb(m).min(l_tms * 3.0)) };
        let r_bgb = radius(l_bgb, giant_t);
        let (l, r) = if t < t_bgb {
            // Eqs. 25–27: across the Hertzsprung gap, geometric in both.
            let tau = (t - t_ms) / (t_bgb - t_ms).max(f64::MIN_POSITIVE);
            (l_tms * pow(l_bgb / l_tms, tau), r_tms * pow(r_bgb / r_tms, tau))
        } else {
            let s = (t - t_bgb) / (t_end - t_bgb);
            let l = l_bgb * pow(GIANT_RISE, s);
            (l, radius(l, giant_t))
        };
        return Evolved { state: State::Giant, mass_solar: initial_mass, luminosity_solar: l, teff_k: teff(l, r), radius_solar: r };
    }
    if m < 8.0 {
        let cooling = ((t - t_end) / 1000.0).max(1e-3);
        let l = (0.01 * pow(cooling / 0.1, -1.4)).min(0.1);
        let r = 0.0126;
        Evolved { state: State::WhiteDwarf, mass_solar: 0.109 * m + 0.394, luminosity_solar: l, teff_k: teff(l, r), radius_solar: r }
    } else if m < 25.0 {
        Evolved { state: State::NeutronStar, mass_solar: 1.4, luminosity_solar: 0.0, teff_k: 0.0, radius_solar: 1.4e-5 }
    } else {
        Evolved { state: State::BlackHole, mass_solar: 0.3 * m, luminosity_solar: 0.0, teff_k: 0.0, radius_solar: 0.0 }
    }
}

/// K, from `L = R² (T/T☉)⁴` in solar units.
fn teff(l: f64, r: f64) -> f64 {
    T_SUN * pow(l / (r * r), 0.25)
}

/// R☉, from the same relation.
fn radius(l: f64, teff: f64) -> f64 {
    libm::sqrt(l) * (T_SUN / teff) * (T_SUN / teff)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tout et al.'s fit, evaluated by hand from its table at Z = 0.02: the ZAMS Sun is fainter
    /// and smaller than today's.
    #[test]
    fn the_zams_sun_matches_the_tout_table() {
        let c = Coefficients::new(0.02);
        assert!((c.l_zams(1.0) - 0.6977).abs() < 5e-4, "{}", c.l_zams(1.0));
        assert!((c.r_zams(1.0) - 0.8883).abs() < 5e-4, "{}", c.r_zams(1.0));
    }

    /// The Sun at 4.57 Gyr comes out near one solar luminosity and radius; Hurley et al. calibrated
    /// against solar models, so a broken coefficient shows here first.
    #[test]
    fn the_sun_today_is_near_one_solar_unit() {
        let e = evolve(1.0, 4.57, 0.0);
        assert_eq!(e.state, State::MainSequence);
        assert!((e.luminosity_solar - 1.0).abs() < 0.1, "L = {}", e.luminosity_solar);
        assert!((e.radius_solar - 1.0).abs() < 0.1, "R = {}", e.radius_solar);
        assert!((e.teff_k - 5772.0).abs() < 200.0, "T = {}", e.teff_k);
    }

    #[test]
    fn lifetimes_fall_with_mass_and_the_sun_lives_about_ten_gyr() {
        let c = Coefficients::new(0.02);
        let t_sun = c.t_ms(1.0) / 1000.0;
        assert!((9.0..12.5).contains(&t_sun), "t_MS(1 M☉) = {t_sun} Gyr");
        let masses = [0.5, 0.8, 1.0, 2.0, 5.0, 10.0, 30.0, 100.0];
        for w in masses.windows(2) {
            assert!(c.t_ms(w[0]) > c.t_ms(w[1]), "{} vs {}", w[0], w[1]);
        }
        assert!(c.t_ms(30.0) < 10.0, "a 30 M☉ star lives a few Myr");
    }

    /// Luminosity grows across the main sequence, and metal-poor stars are hotter and brighter at
    /// a given mass.
    #[test]
    fn main_sequence_stars_brighten_and_metal_poor_ones_run_hotter() {
        for m in [0.3, 0.8, 1.0, 1.5, 3.0, 10.0] {
            let c = Coefficients::new(0.02);
            let t = c.t_ms(m);
            let (l0, _) = c.main_sequence(m, 0.0);
            let (l1, _) = c.main_sequence(m, 0.95 * t);
            assert!(l1 > l0, "M = {m}");
            assert!((l0 / c.l_zams(m) - 1.0).abs() < 1e-9, "starts on the ZAMS at M = {m}");
        }
        let rich = evolve(1.0, 0.0, 0.0);
        let poor = evolve(1.0, 0.0, -1.5);
        assert!(poor.teff_k > rich.teff_k + 300.0 && poor.luminosity_solar > rich.luminosity_solar);
    }

    #[test]
    fn every_mass_and_metallicity_gives_finite_stars_at_every_age() {
        for feh in [-2.5, -1.0, 0.0, 0.2] {
            for i in 0..60 {
                let m = 0.08 * (150.0f64 / 0.08).powf(i as f64 / 59.0);
                for age in [0.0, 0.01, 0.1, 1.0, 5.0, 13.0] {
                    let e = evolve(m, age, feh);
                    for v in [e.luminosity_solar, e.teff_k, e.radius_solar, e.mass_solar] {
                        assert!(v.is_finite() && v >= 0.0, "M = {m}, age = {age}, [Fe/H] = {feh}: {e:?}");
                    }
                    if e.state == State::MainSequence {
                        assert!(e.teff_k > 1500.0 && e.teff_k < 70_000.0, "{m} {age} {feh}: {e:?}");
                    }
                }
            }
        }
    }
}
