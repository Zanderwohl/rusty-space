//! What a giant looks like from orbit, from what it is: cloud-top temperature picks what
//! condenses (Sudarsky et al. 2000), envelope metals set methane and belt stain, ultraviolet sets
//! how fast stain and haze are made, and spin sets the band count. Spectra are per band and per
//! layer, so every band mapping and the survey read the same thing the graph paints. The
//! constants are fitted to the four solar giants' measured colors; the ordering matters, not the
//! third digit. See `lightcone/docs/07-rendering.md`.

use em_spectra::{BANDS, Band};

use crate::surface::Surface;

/// In the order the shader mixes them.
pub const LAYERS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Zone,
    Belt,
    Storm,
    Polar,
}

impl Layer {
    pub const ALL: [Layer; LAYERS] = [Layer::Zone, Layer::Belt, Layer::Storm, Layer::Polar];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Giant {
    /// Per band, per [`Layer`]. Zero in the emissive bands.
    pub layers: [[f32; BANDS]; LAYERS],
    pub colors: Colors,
    /// Belt-zone pairs pole to pole: Jupiter's is about six.
    pub bands: f32,
    /// `[0, 1]`.
    pub contrast: f32,
    /// `[0, 1]`.
    pub turbulence: f32,
    /// `[0, 1]`.
    pub storms: f32,
    /// Share of the sphere under polar haze.
    pub polar: f32,
    /// Band phase, `[0, 1]` of a cycle.
    pub shift: f32,
    /// How much great spot, and how stained the ovals are, `[0, 1]`.
    pub spot: f32,
}

/// The graph's colors, Oklch `(L, C, hue in degrees)`. `tint` is a belt stained twice as deep.
/// These are the look, not the measurement: see [`look`] and [`painted`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub zone: [f32; 3],
    pub belt: [f32; 3],
    pub tint: [f32; 3],
    pub storm: [f32; 3],
    pub polar: [f32; 3],
}

impl Giant {
    /// Share of the sphere each layer covers, fitted to the graph; a client test holds it there.
    pub fn shares(&self) -> [f32; LAYERS] {
        let polar = self.polar;
        let storm = (1.0 - polar) * storm_share(self.storms);
        let belt = (1.0 - polar - storm) * BELTED * self.contrast * (1.0 - 0.4 * self.turbulence);
        [1.0 - polar - storm - belt, belt, storm, polar]
    }

    /// What a survey reads, in [`Band`] order.
    pub fn reflectance(&self) -> [f64; BANDS] {
        let shares = self.shares();
        std::array::from_fn(|b| {
            self.layers.iter().zip(shares).map(|(r, s)| f64::from(r[b] * s)).sum()
        })
    }
}

/// Share of the bands the graph's belt pattern covers before contrast and wisps, measured.
const BELTED: f32 = 0.42;

/// Share of the bands the graph's ovals cover, measured.
pub fn storm_share(storms: f32) -> f32 {
    0.005 + 0.045 * storms.clamp(0.0, 1.0)
}

/// `None` was not stated, and the rules stand in.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub mass_kg: f64,
    pub radius_m: f64,
    /// Cloud-top temperature, kelvin.
    pub effective_k: f64,
    /// Kelvin; stands for how much starlight reaches it.
    pub equilibrium_k: f64,
    /// `[M/H]` of the envelope, dex over solar.
    pub metals: Option<f64>,
    pub runaway: bool,
    pub star_feh: f64,
    pub star_teff_k: f64,
    /// One turn, seconds.
    pub spin_s: Option<f64>,
}

pub const JUPITER_MASS: f64 = 1.898e27;
const JUPITER_RADIUS: f64 = 6.9911e7;
const JUPITER_SPIN_S: f64 = 35_730.0;

/// The generator's tag for the envelope's `[M/H]`, dex.
pub const METALS: &str = "Metals:";

pub fn metals_tag(metals: f64) -> String {
    format!("{METALS}{metals}")
}

pub fn metals_of(tags: &[String]) -> Option<f64> {
    tags.iter().find_map(|t| t.strip_prefix(METALS)?.parse().ok())
}

/// Envelope metals over the star's: `4 (M / M_J)^-0.9`, through Jupiter's 4, Saturn's 10 and the
/// ice giants' ~80 (Welbanks et al. 2019). A runaway core's envelope is nebular gas however
/// small it stayed, so it is held under 12; otherwise over 30. That split is what keeps a small
/// gas giant and an ice giant apart by color.
pub fn enrichment(mass_kg: f64, runaway: bool) -> f64 {
    let law = 4.0 * (mass_kg / JUPITER_MASS).max(1.0e-6).powf(-0.9);
    if runaway { law.clamp(1.0, 12.0) } else { law.clamp(30.0, 150.0) }
}

pub fn metals(mass_kg: f64, runaway: bool, star_feh: f64) -> f64 {
    star_feh + enrichment(mass_kg, runaway).log10()
}

impl Inputs {
    pub fn from_tags(
        surface: Surface,
        mass_kg: f64,
        radius_m: f64,
        equilibrium_k: f64,
        star_feh: f64,
        star_teff_k: f64,
        spin_s: Option<f64>,
        tags: &[String],
    ) -> Self {
        let stated = crate::worlds::Stated::from_tags(tags);
        Self {
            // Untagged bodies fall back to the class's mass threshold.
            runaway: if stated.atmosphere.is_some() { stated.gas_giant } else { surface == Surface::GasGiant },
            mass_kg,
            radius_m,
            effective_k: surface.effective_temperature(equilibrium_k),
            equilibrium_k,
            metals: metals_of(tags),
            star_feh,
            star_teff_k,
            spin_s,
        }
    }
}

/// Per-body variation, uniform in `[0, 1]` and keyed by id so it is stable.
#[derive(Clone, Copy, Debug)]
pub struct Variety([f32; 9]);

pub fn variety(id: &str) -> Variety {
    // Salted apart from `worlds::varied` and the climate's.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    Variety(std::array::from_fn(|k| crate::rng::uniform(crate::rng::hash(&[h, 0x61a, k as u64])) as f32))
}

pub fn of(id: &str, inputs: &Inputs) -> Giant {
    let derived = derived(inputs, variety(id));
    measured(id, derived).unwrap_or(derived)
}

// Per band, B V R I K.

const AMMONIA: [f32; 5] = [0.84, 0.88, 0.9, 0.88, 0.56];
const WATER: [f32; 5] = [0.84, 0.87, 0.87, 0.82, 0.3];
const SILICATE: [f32; 5] = [0.38, 0.48, 0.58, 0.64, 0.68];
/// A clear hydrogen column seen from above.
const RAYLEIGH: [f32; 5] = [0.60, 0.30, 0.14, 0.05, 0.008];
const HAZE: [f32; 5] = [0.58, 0.64, 0.67, 0.68, 0.6];
/// Dark in the visible, and bright in K because it is above the methane.
const POLAR_HAZE: [f32; 5] = [0.24, 0.32, 0.38, 0.44, 0.5];

// Absorption per unit column.

const METHANE: [f32; 5] = [0.0, 0.03, 0.12, 0.42, 1.8];
const CHROMOPHORE: [f32; 5] = [1.0, 0.55, 0.22, 0.05, 0.0];

/// `hue` runs from sulfur yellow to a deep red-brown.
fn chromophore(hue: f32) -> [f32; 5] {
    let [b, v, r, i, k] = CHROMOPHORE;
    let deeper = 0.55 + 0.7 * (hue - 0.5);
    [b, deeper, r * deeper / v, i, k]
}

/// Sodium and potassium, pressure-broadened across the visible.
const ALKALI: [f32; 5] = [0.7, 1.6, 1.3, 0.8, 0.05];

/// A layer's place in the column.
struct Part {
    deck: Deck,
    /// Gas column over it, relative.
    depth: f32,
    /// Chromophore column, in belts.
    stain: f32,
    /// Share of polar haze on top.
    haze: f32,
}

#[derive(Clone, Copy)]
enum Deck {
    Zone,
    Belt,
    Full,
}

/// `(ammonia, water, clear, alkali, silicate)`, summing to one. Smooth, so giants either side of
/// a threshold do not look like two classes.
fn regimes(k: f64) -> [f32; 5] {
    let up = |a: f64, b: f64| {
        let t = ((k - a) / (b - a)).clamp(0.0, 1.0);
        (t * t * (3.0 - 2.0 * t)) as f32
    };
    let [a, b, c, d] = [up(150.0, 210.0), up(320.0, 420.0), up(750.0, 950.0), up(1300.0, 1600.0)];
    [1.0 - a, a * (1.0 - b), b * (1.0 - c), c * (1.0 - d), d]
}

pub fn derived(inputs: &Inputs, variety: Variety) -> Giant {
    let Variety([a, b, c, d, e, f, g, h, i]) = variety;
    let t = inputs.effective_k.max(10.0);
    let w = regimes(t);
    let [ammonia, water, clear, alkali, silicate] = w;
    let metals = inputs.metals.unwrap_or_else(|| metals(inputs.mass_kg, inputs.runaway, inputs.star_feh));
    let x = 10f64.powf(metals).clamp(0.05, 300.0) as f32;

    // Ultraviolet against Jupiter's: T_eq^4 for distance, times Wien at 250 nm for the star,
    // softened because a red dwarf's chromosphere makes more than its photosphere.
    let wien = |k: f64| (-57_552.0 / k.max(2000.0)).exp();
    let uv = (inputs.equilibrium_k / 122.0).powi(4) * (wien(inputs.star_teff_k) / wien(5772.0)).powf(0.6);
    let photochemistry = uv.clamp(1.0e-3, 30.0).powf(0.3) as f32;

    // A colder deck condenses further down.
    let sink = (124.0 / t as f32).clamp(0.5, 3.0).powf(1.5);
    let sink = (ammonia + water) * sink + (1.0 - ammonia - water);
    // Hot, carbon is carbon monoxide.
    let methane = 0.106 * x.powf(0.7) * (1.0 - smooth(800.0, 1100.0, t)) * sink;
    // Below about 80 K the top deck is hydrogen sulfide, which nothing stains.
    let stained_deck = smooth(60.0, 110.0, t);
    // Capped, or under a hot star belts and ovals go black.
    let stain = (0.6 * (0.6 + 0.8 * a) * photochemistry * (x / 4.0).sqrt().min(2.5) * (ammonia * stained_deck + 0.35 * water))
        .min(1.5);
    let gas = (0.15 * (sink - 1.0)).clamp(0.0, 0.5);
    let dim = sink.powf(-0.15);
    let alkali_depth = 1.6 * (alkali + 0.05 * silicate + 0.3 * clear) * (x / 4.0).powf(0.3);
    let cold = 1.0 - smooth(55.0, 135.0, t);
    let haze = ammonia * (0.1 + 0.4 * cold * photochemistry.min(1.0)) + 0.08 * (water + silicate) + 0.02 * (clear + alkali);

    // Where nothing condenses, a zone is a thin haze.
    let cloud: [f32; 5] = std::array::from_fn(|k| {
        ammonia * AMMONIA[k] + water * WATER[k] + silicate * SILICATE[k] + (clear + alkali) * HAZE[k]
    });
    let zone_cover = 0.95 * ammonia + 0.98 * water + 0.12 * clear + 0.1 * alkali + 0.88 * silicate;
    let belt_cover = 0.78 * ammonia + 0.85 * water + 0.0 * clear + 0.0 * alkali + 0.5 * silicate;

    let chromophore = chromophore(i);
    let layer = |part: Part| -> [f32; BANDS] {
        let cover = match part.deck {
            Deck::Zone => zone_cover,
            Deck::Belt => belt_cover,
            Deck::Full => zone_cover.max(0.97 * (ammonia + water + silicate)),
        };
        let gray = (-0.35 * part.depth * sink.min(2.0)).exp();
        let mut out = [0.0; BANDS];
        for k in 0..5 {
            let stained = (-stain * part.stain * chromophore[k]).exp();
            let under = (1.0 - gas) * (cover * cloud[k] * stained + (1.0 - cover) * RAYLEIGH[k]) + gas * RAYLEIGH[k];
            let under = under * gray * (-methane * part.depth * METHANE[k]).exp() * (-alkali_depth * ALKALI[k]).exp();
            // The haze is above most of the methane.
            let above = HAZE[k] * (-0.12 * methane * METHANE[k]).exp() * (-0.5 * stain * chromophore[k]).exp();
            let top = haze * above + (1.0 - haze) * under;
            let polar = POLAR_HAZE[k] * (-0.12 * methane * METHANE[k]).exp();
            out[k] = (part.haze * polar + (1.0 - part.haze) * top) * dim;
        }
        out
    };
    let zone = layer(Part { deck: Deck::Zone, depth: 0.35, stain: 0.25 + 0.2 * b, haze: 0.0 });
    let belt = layer(Part { deck: Deck::Belt, depth: 1.0, stain: 1.0, haze: 0.0 });
    let tint = layer(Part { deck: Deck::Belt, depth: 1.0, stain: 2.2, haze: 0.0 });
    let spot = f32::from(h < 0.35) * (0.4 + 0.6 * e);
    let storm = layer(Part { deck: Deck::Full, depth: 0.08, stain: 0.08 + 1.2 * spot, haze: 0.0 });
    let polar = layer(Part { deck: Deck::Zone, depth: 0.6, stain: 0.6, haze: 0.6 * photochemistry.min(1.0) });

    // Rhines: jets go as the square root of the equator's speed.
    let spin = inputs.spin_s.unwrap_or(JUPITER_SPIN_S).max(3600.0);
    let turning = (inputs.radius_m / JUPITER_RADIUS) * (JUPITER_SPIN_S / spin);
    let bands = (5.7 * turning.sqrt() as f32 * (0.85 + 0.3 * c)).clamp(1.2, 12.0);

    // A cold deck's haze veils its belts.
    let veiled = 1.0 - 0.7 * cold * ammonia;
    let contrast = ((0.95 * ammonia + 0.45 * water + 0.35 * clear + 0.35 * alkali + 0.7 * silicate) * veiled
        * (0.75 + 0.5 * d))
        .clamp(0.05, 1.0);
    let turbulence = ((0.35 + 0.65 * (inputs.mass_kg / JUPITER_MASS).sqrt() as f32).min(1.0) * (0.6 + 0.6 * f)).clamp(0.1, 1.0);
    let storms = (contrast * (0.4 + 0.8 * g)).clamp(0.0, 1.0);
    let polar_share = (0.06 + 0.1 * a) * (ammonia + water);

    Giant {
        layers: [zone, belt, storm, polar],
        colors: {
            let zone_c = display(&zone);
            Colors {
                zone: painted(look(zone_c, zone_c)),
                belt: painted(look(display(&belt), zone_c)),
                tint: painted(look(display(&tint), zone_c)),
                // Chroma only, or a stained oval goes black.
                storm: painted(look(display(&storm), [0.0; 3])),
                polar: painted(look(display(&polar), zone_c)),
            }
        },
        bands,
        contrast,
        turbulence,
        storms,
        polar: polar_share,
        shift: d * 0.7 + e * 0.3,
        spot,
    }
}

fn smooth(a: f64, b: f64, x: f64) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

/// `(R, V, B)` as Oklch: the natural mapping under white light.
pub fn display(r: &[f32; BANDS]) -> [f32; 3] {
    oklch([r[Band::R.index()], r[Band::V.index()], r[Band::B.index()]].map(|c| c.clamp(0.0, 1.0)))
}

/// What to paint so that under the Sun it is seen as `seen`. The natural mapping's Sun is
/// `(1.37, 1, 1.04)`, R being the widest band; other stars still tint through the light.
fn painted(seen: [f32; 3]) -> [f32; 3] {
    let sun = [Band::R, Band::V, Band::B].map(|b| em_spectra::blackbody::band_radiance(b, 5772.0) as f32);
    let luma = 0.2126 * sun[0] + 0.7152 * sun[1] + 0.0722 * sun[2];
    let rgb = linear(seen);
    oklch(std::array::from_fn(|c| (rgb[c] * luma / sun[c]).clamp(0.0, 1.0)))
}

/// How a giant is seen rather than measured, as photographs raise it: measured belts are nearly
/// gray and vanish in a surface's tone window. Only the cubemap is raised; the spectra are not.
fn look([l, c, h]: [f32; 3], [zone_l, ..]: [f32; 3]) -> [f32; 3] {
    let l = if l < zone_l { zone_l + 2.5 * (l - zone_l) } else { l };
    // Raised as far, a dark color reads as neon.
    [l.max(0.0), (2.4 * c).min(c.max(0.2 * l.min(0.85))), h]
}

/// Oklch to linear sRGB, after Ottosson.
fn linear([l, c, h]: [f32; 3]) -> [f32; 3] {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
}

/// Linear sRGB to Oklch, after Ottosson.
fn oklch([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let [l, m, s] = [l.cbrt(), m.cbrt(), s.cbrt()];
    let lab_l = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let lab_a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let lab_b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    [lab_l, lab_a.hypot(lab_b), lab_b.atan2(lab_a).to_degrees().rem_euclid(360.0)]
}

/// The rule's own colors, with the layout each is known by.
fn measured(id: &str, derived: Giant) -> Option<Giant> {
    Some(match id {
        "Jupiter" => Giant { bands: 5.7, contrast: 0.9, turbulence: 0.9, storms: 0.45, spot: 1.0, shift: 0.0, ..derived },
        "Saturn" => Giant { bands: 4.5, contrast: 0.35, turbulence: 0.3, storms: 0.1, spot: 0.0, ..derived },
        "Uranus" => Giant { bands: 2.0, contrast: 0.08, turbulence: 0.15, storms: 0.05, spot: 0.0, ..derived },
        "Neptune" => Giant { bands: 2.6, contrast: 0.25, turbulence: 0.4, storms: 0.4, spot: 0.0, ..derived },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EARTH_MASS: f64 = 5.9722e24;

    fn inputs(id: &str) -> Inputs {
        // (mass kg, radius m, equilibrium K, spin s, surface)
        let (mass, radius, eq, spin, surface) = match id {
            "Jupiter" => (JUPITER_MASS, JUPITER_RADIUS, 122.0, JUPITER_SPIN_S, Surface::GasGiant),
            "Saturn" => (5.683e26, 5.8232e7, 90.0, 38_362.0, Surface::GasGiant),
            "Uranus" => (8.681e25, 2.5559e7, 64.0, 62_064.0, Surface::IceGiant),
            "Neptune" => (1.024e26, 2.4764e7, 51.0, 57_996.0, Surface::IceGiant),
            _ => unreachable!(),
        };
        Inputs::from_tags(surface, mass, radius, eq, 0.0, 5772.0, Some(spin), &[])
    }

    fn giant(id: &str) -> Giant {
        derived(&inputs(id), variety(id))
    }

    fn ratio(r: [f64; BANDS], over: Band, under: Band) -> f64 {
        r[over.index()] / r[under.index()]
    }

    /// A belt's R over B.
    fn redness(g: &Giant) -> f32 {
        let belt = g.layers[Layer::Belt as usize];
        belt[Band::R.index()] / belt[Band::B.index()]
    }

    /// Given the four solar giants' inputs, the rule lands on their measured colors.
    #[test]
    fn the_rule_reaches_the_solar_systems_giants() {
        let mut misses = Vec::new();
        for id in ["Jupiter", "Saturn", "Uranus", "Neptune"] {
            let got = giant(id).reflectance();
            let want = crate::worlds::for_body(id).unwrap().reflectance;
            eprintln!("{id}: {:.3?} against {:.3?}", &got[..5], &want[..5]);
            for (over, under, tolerance) in [(Band::R, Band::B, 0.2), (Band::K, Band::R, 0.35), (Band::I, Band::V, 0.2)] {
                let (g, w) = (ratio(got, over, under), ratio(want, over, under));
                if (g / w - 1.0).abs() > tolerance {
                    misses.push(format!("{id} {over:?}/{under:?}: {g:.3} against {w:.3}"));
                }
            }
            let v = Band::V.index();
            if (got[v] / want[v] - 1.0).abs() > 0.25 {
                misses.push(format!("{id} V: {:.3} against {:.3}", got[v], want[v]));
            }
        }
        assert!(misses.is_empty(), "{misses:#?}");
    }

    /// Saturn's belts are fainter than Jupiter's; an ice giant is blue and nearly black in K.
    #[test]
    fn the_giants_are_ordered_as_they_are_seen() {
        let [jupiter, saturn, uranus] = ["Jupiter", "Saturn", "Uranus"].map(giant);
        assert!(saturn.contrast < jupiter.contrast * 0.7, "{} {}", saturn.contrast, jupiter.contrast);
        let blue = |g: &Giant| ratio(g.reflectance(), Band::B, Band::R);
        assert!(blue(&uranus) > 1.2 && blue(&jupiter) < 1.0);
        assert!(ratio(uranus.reflectance(), Band::K, Band::V) < 0.2);
    }

    fn hot(k: f64) -> Giant {
        let eq = k / 1.0;
        derived(
            &Inputs {
                mass_kg: JUPITER_MASS,
                radius_m: 1.2 * JUPITER_RADIUS,
                effective_k: k,
                equilibrium_k: eq,
                metals: None,
                runaway: true,
                star_feh: 0.0,
                star_teff_k: 5772.0,
                spin_s: Some(3.0 * 86_400.0),
            },
            variety("hot"),
        )
    }

    /// Sudarsky's sequence: water cloud brightest, clear column blue, alkali dark, silicate bright.
    #[test]
    fn warmer_cloud_tops_run_through_the_five_classes() {
        let v = |g: Giant| g.reflectance()[Band::V.index()];
        let [ammonia, water, clear, alkali, silicate] = [124.0, 260.0, 600.0, 1100.0, 1900.0].map(hot);
        assert!(v(water) > v(ammonia), "water cloud outshines ammonia");
        assert!(ratio(clear.reflectance(), Band::B, Band::R) > 2.0, "a clear column is blue");
        assert!(v(alkali) < 0.08, "alkali vapor is dark: {}", v(alkali));
        assert!(v(silicate) > 2.0 * v(alkali), "silicate cloud brightens it again");
        assert!(ratio(silicate.reflectance(), Band::K, Band::V) > 0.8, "and there is no methane to darken K");
    }

    /// A metal-rich star's giant is darker in K and redder in its belts.
    #[test]
    fn metals_deepen_the_methane_and_the_stain() {
        let at = |feh: f64| {
            let mut i = inputs("Saturn");
            i.star_feh = feh;
            derived(&i, variety("x"))
        };
        let (poor, rich) = (at(-1.0), at(0.4));
        let k = |g: &Giant| g.reflectance()[Band::K.index()];
        assert!(k(&rich) < 0.8 * k(&poor), "{} {}", k(&rich), k(&poor));
        assert!(redness(&rich) > redness(&poor), "richer belts are redder");
        assert!(metals(JUPITER_MASS, true, 0.0) > 0.5 && metals(16.0 * EARTH_MASS, false, 0.0) > 1.6);
        assert!(metals(16.0 * EARTH_MASS, true, 0.0) < 1.1, "a runaway is diluted however small");
    }

    /// A hot star stains belts more than a red dwarf does.
    #[test]
    fn a_hot_star_stains_the_belts_and_a_cool_one_leaves_them_pale() {
        let at = |teff: f64| {
            let mut i = inputs("Jupiter");
            i.star_teff_k = teff;
            redness(&derived(&i, variety("x")))
        };
        assert!(at(7000.0) > at(5772.0) && at(5772.0) > at(3500.0));
    }

    #[test]
    fn a_fast_big_giant_has_more_bands() {
        let jupiter = giant("Jupiter").bands;
        assert!(giant("Uranus").bands < jupiter * 0.7);
        assert!(hot(1100.0).bands < 3.0, "a tidally locked giant has few, broad bands");
    }

    #[test]
    fn the_metals_tag_survives_the_round_trip() {
        for m in [-1.2, 0.0, 0.63, 2.1] {
            assert_eq!(metals_of(&[metals_tag(m)]), Some(m));
        }
    }

    #[test]
    fn the_shares_cover_the_sphere() {
        for id in ["Jupiter", "Saturn", "Uranus", "Neptune", "x"] {
            let s = of(id, &inputs(if id == "x" { "Jupiter" } else { id })).shares();
            assert!((s.iter().sum::<f32>() - 1.0).abs() < 1e-5 && s.iter().all(|v| *v >= 0.0), "{s:?}");
        }
    }

    #[test]
    fn a_gray_is_its_own_lightness() {
        let [l, c, _] = oklch([0.5, 0.5, 0.5]);
        assert!((l - 0.5f32.cbrt()).abs() < 1e-3 && c < 1e-3);
        let back = linear(oklch([0.6, 0.3, 0.1]));
        assert!(back.iter().zip([0.6, 0.3, 0.1]).all(|(a, b)| (a - b).abs() < 1e-3), "{back:?}");
    }
}
