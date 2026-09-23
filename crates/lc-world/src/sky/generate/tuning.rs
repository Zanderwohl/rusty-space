//! Every number the generator draws from, in one place.
//!
//! Split out so a system's architecture can be swept rather than recompiled: the plots in
//! `lightcone/docs/26-system-generation.md` are made by varying these and running the same
//! code. [`Tuning::default`] is what the game ships.
//!
//! Where a figure is a measurement it says so. Where it is a choice it says that too, because
//! this is a game and the choice is usually to be more generous than the galaxy is.

/// The disc a system condenses out of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disc {
    /// Temperature at which silicates condense, kelvin. Inside this radius the disc holds no
    /// solids, so it is where the innermost body can be.
    pub sublimation_k: f64,
    /// Temperature at which water ice condenses, kelvin. The snow line, and the single most
    /// consequential radius in the system: outside it every body gets ices as well as rock,
    /// which is roughly four times the solid mass and is what lets a core grow big enough to
    /// take an envelope.
    pub snow_k: f64,
    /// Equilibrium temperatures bounding the habitable zone, kelvin, warm end first.
    ///
    /// Zero-albedo equilibrium, which is what [`super::disc::Disc::radius_at`] inverts, so a
    /// dim star's zone follows from its own light with no separate formula. The default is the
    /// optimistic zone -- recent Venus to early Mars, 0.75 to 1.77 astronomical units for the
    /// Sun -- rather than the conservative one.
    pub habitable_k: (f64, f64),
    /// Disc outer edge, in snow lines. Fifteen puts the Sun's at 40 astronomical units, which
    /// is the Kuiper belt's outer edge.
    pub outer_over_snow: f64,
    pub outer_jitter: (f64, f64),
    /// Solid mass in the disc of a solar-mass star at solar metallicity, Earth masses.
    ///
    /// The minimum-mass solar nebula is about 50. Higher on purpose: a disc that only just
    /// built the solar system builds very little anywhere else.
    pub solid_earths: f64,
    /// How the disc mass scales with the star's, as a power. Observed discs run near linear.
    pub solids_per_stellar_mass: f64,
    /// Spread in the disc's solid mass, dex.
    pub solid_spread_dex: f64,
    /// Solid surface density falls as `r^-index`. The minimum-mass nebula's value.
    pub surface_index: f64,
    /// How much more solid there is past the snow line, where ices condense too.
    pub ice_boost: f64,
}

/// How the disc is cut into bodies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ladder {
    /// Where the innermost rung sits, in sublimation radii.
    pub first_rung: (f64, f64),
    /// Ratio between neighbouring rungs. Drawn per step, so a system's spacing is not one
    /// number repeated.
    pub spacing: (f64, f64),
    pub max_rungs: usize,
    /// Share of a feeding zone's solids that ends up in the body, drawn per rung.
    pub efficiency: (f64, f64),
    /// Radius out to which a rung finishes assembling, in snow lines.
    ///
    /// Accretion slows as the cube of the orbit, so past this the disc runs out of time and
    /// leaves its solids where they lie. That leftover *is* the trans-planetary belt, which is
    /// why the Kuiper analogue costs nothing to place: mass conservation puts it there.
    pub growth_over_snow: f64,
    /// Core mass above which a body beyond the snow line holds an ice envelope, Earth masses.
    pub ice_giant_core_earths: f64,
    /// Core mass at which gas accretion runs away, Earth masses.
    ///
    /// The critical core mass is about ten in the standard picture, and lower in a colder or
    /// denser disc. Lowering it is the most direct way to make giants common.
    pub runaway_core_earths: f64,
    /// Envelope mass as a multiple of the core, log-uniform.
    pub envelope: (f64, f64),
    /// Heaviest a planet may be, Jupiter masses. Above thirteen it burns deuterium and is a
    /// brown dwarf rather than a planet.
    pub heaviest_jupiters: f64,
    /// Share of giants that migrate inward to somewhere between the disc's inner edge and the
    /// snow line, taking out everything they cross.
    pub migrating_fraction: f64,
    /// Width of a giant's resonance web, as `C` in `a(1 +- C mu^0.2)`.
    ///
    /// Not a clearing radius -- the giant's own chaotic zone is far narrower. This is the reach
    /// of its mean-motion resonances, which is what stops a belt accreting. Jupiter's `mu` puts
    /// the inner edge at 0.40 of its own axis, or 2.1 astronomical units, which is where the
    /// asteroid belt starts.
    pub resonance_reach: f64,
}

/// What a planet turns out to be, given where it formed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct World {
    /// Exosphere temperature over equilibrium temperature. Earth's 255 K equilibrium sits
    /// under a 1000 K exosphere, which is what actually sets what escapes.
    pub exosphere_factor: f64,
    /// Escape velocity over thermal speed at which a gas is held for the age of the system.
    /// Six is the textbook figure and it puts hydrogen off Earth and nitrogen on it.
    pub retention: f64,
    /// Mass at which a body has a core hot enough to convect, Earth masses.
    pub dynamo_mass_earths: f64,
    /// Rotation period beyond which a dynamo stalls, seconds. Venus turns in 243 days and has
    /// no field.
    pub dynamo_spin_s: f64,
    /// Chance an airless-by-stripping planet keeps its air anyway, and its opposite: nothing
    /// here is a hard gate.
    pub dynamo_floor: f64,
    /// How much of the icy reservoir reaches an inner planet, when a giant is there to throw it.
    pub delivered_water: (f64, f64),
    /// Rotation, seconds, log-uniform by class.
    pub giant_spin_s: (f64, f64),
    pub rocky_spin_s: (f64, f64),
    /// Obliquity: a gaussian at this sigma, with a tail that lands on its side.
    pub typical_obliquity_rad: f64,
    pub tumbled_chance: f64,
}

/// Moons, of both origins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moons {
    /// Mass of a giant's regular satellite system over the planet's own.
    ///
    /// Measured, and remarkably constant: Jupiter's Galileans, Saturn's mid-sized moons and
    /// Uranus's all come to about a ten-thousandth of their planet. That one number is what
    /// makes a generated giant's retinue the right size without any other tuning.
    pub regular_mass_ratio: f64,
    pub regular_count: (u32, u32),
    /// Inner bound on a regular moon, in planetary radii.
    pub inner_radii: f64,
    /// Outer bound, as a share of the Hill radius. Regular satellites form in a disc well
    /// inside it: the Galileans sit within a fiftieth of Jupiter's.
    pub regular_outer_hill: f64,
    /// Chance a rocky planet took a giant impact and kept the debris. Luna is an eighth of a
    /// percent of Earth by mass and nothing else in the inner system has anything like it.
    pub impact_moon_chance: f64,
    pub impact_mass_ratio: (f64, f64),
    /// Most captured irregulars a planet may hold, and the power law that decides how many.
    ///
    /// Jupiter has ninety-odd known irregulars and the count is still climbing. They are
    /// captured rather than formed, so they sit far out, at every inclination, and more than
    /// half of them go backwards.
    pub irregular_most: u32,
    pub irregular_index: f64,
    /// Where an irregular sits, as a share of the Hill radius.
    pub irregular_hill: (f64, f64),
    pub irregular_eccentricity: (f64, f64),
    /// Share of irregulars on retrograde orbits. Two thirds of Jupiter's are.
    pub retrograde_share: f64,
    /// Radius of an irregular, meters, log-uniform.
    pub irregular_radius_m: (f64, f64),
    /// Densities a satellite may have: icy near 1200, rocky near 3500.
    pub density: (f64, f64),
}

/// Belts, the Kuiper analogue and the Oort cloud.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Belts {
    /// Share of a sterilised rung's solids still there to be seen. The asteroid belt holds
    /// about a two-thousandth of what its feeding zone started with; the rest was thrown out.
    pub belt_survival: f64,
    /// The same for the trans-planetary belt, which was stirred far less.
    pub kuiper_survival: f64,
    /// Bulk density of a belt element, kg/m^3.
    pub element_density: f64,
    /// Share of the icy reservoir a system's giants throw into the Oort cloud.
    ///
    /// A system with no giant has no scatterer, so its cloud is nearly empty -- which is a
    /// statement about where comets come from rather than a rule invented here.
    pub oort_efficiency: f64,
    /// Giant mass, in Jupiters, at which scattering is as efficient as it gets.
    pub oort_saturation_jupiters: f64,
    pub oort_au: (f64, f64),
    pub oort_eccentricity: (f64, f64),
    /// Geometric cross-section a population presents per kilogram it holds, m^2/kg.
    ///
    /// Mass says how much is there; this says how much of it is surface, which is what a
    /// telescope measures. The three differ by orders of magnitude because their size
    /// distributions do: the asteroid belt's mass is in a handful of large bodies and the Oort
    /// cloud's is in a great many small ones. Measured from the real three.
    pub belt_area_per_kg: f64,
    pub kuiper_area_per_kg: f64,
    pub oort_area_per_kg: f64,
    /// Radius of one element, meters. Sets what a single transit looks like, not the total.
    pub belt_element_m: f64,
    pub kuiper_element_m: f64,
    pub oort_element_m: f64,
    /// How many dwarf planets condense out of the trans-planetary belt.
    pub dwarfs: (u32, u32),
}

/// The generator's knobs, whole.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    pub disc: Disc,
    pub ladder: Ladder,
    pub world: World,
    pub moons: Moons,
    pub belts: Belts,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            disc: Disc {
                sublimation_k: 1500.0,
                snow_k: 170.0,
                habitable_k: (321.0, 209.0),
                outer_over_snow: 15.0,
                outer_jitter: (0.7, 1.5),
                solid_earths: 75.0,
                solids_per_stellar_mass: 1.0,
                solid_spread_dex: 0.25,
                surface_index: 1.5,
                ice_boost: 4.0,
            },
            ladder: Ladder {
                first_rung: (1.0, 4.0),
                spacing: (1.35, 2.1),
                max_rungs: 24,
                efficiency: (0.35, 1.0),
                growth_over_snow: 4.5,
                ice_giant_core_earths: 3.0,
                runaway_core_earths: 8.0,
                envelope: (2.0, 60.0),
                heaviest_jupiters: 13.0,
                migrating_fraction: 0.08,
                resonance_reach: 2.4,
            },
            world: World {
                exosphere_factor: 3.9,
                retention: 6.0,
                dynamo_mass_earths: 0.5,
                dynamo_spin_s: 20.0 * 86_400.0,
                dynamo_floor: 0.1,
                delivered_water: (1.0e-4, 5.0e-3),
                giant_spin_s: (7.0 * 3600.0, 20.0 * 3600.0),
                rocky_spin_s: (4.0 * 3600.0, 250.0 * 86_400.0),
                typical_obliquity_rad: 0.35,
                tumbled_chance: 0.1,
            },
            moons: Moons {
                regular_mass_ratio: 1.0e-4,
                regular_count: (2, 6),
                inner_radii: 2.5,
                regular_outer_hill: 0.05,
                impact_moon_chance: 0.15,
                impact_mass_ratio: (1.0e-3, 2.0e-2),
                irregular_most: 100,
                irregular_index: 1.8,
                irregular_hill: (0.1, 0.5),
                irregular_eccentricity: (0.1, 0.7),
                retrograde_share: 0.65,
                irregular_radius_m: (1.0e3, 6.0e4),
                density: (1200.0, 3500.0),
            },
            belts: Belts {
                belt_survival: 5.0e-4,
                kuiper_survival: 0.002,
                element_density: 2000.0,
                oort_efficiency: 0.3,
                oort_saturation_jupiters: 1.0,
                oort_au: (2_000.0, 100_000.0),
                oort_eccentricity: (0.6, 0.95),
                belt_area_per_kg: 1.0e-9,
                kuiper_area_per_kg: 1.0e-8,
                oort_area_per_kg: 3.7e-7,
                belt_element_m: 1.0e3,
                kuiper_element_m: 5.0e4,
                oort_element_m: 1.0e3,
                dwarfs: (0, 4),
            },
        }
    }
}
