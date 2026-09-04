import json, math
from bodies import all_bodies, BARYCENTRIC

G = 6.6743015e-11
phys = json.load(open("phys.json"))
fits = json.load(open("fits.json"))
elements = json.load(open("elements.json"))

# Radii the repo already carried. They were stored as DIAMETERS under a `radius` field;
# halved here. JPL publishes no size for these three.
EXISTING_DIAMETER_KM = {"eris": 2326.0, "Sedna": 906.0, "dysnomia": 615.0}

# Masses the repo already carried, for bodies where JPL publishes no GM. Eris's matters:
# it is what Dysnomia's orbit is solved against.
EXISTING_MASS_KG = {"eris": 1.6466e22, "dysnomia": 8.2e19, "Sedna": 2.0e21}

# Bulk densities, g/cm^3, used only to estimate a mass where JPL publishes no GM.
DENSITY = {"Comet": 0.5, "TNO": 1.5, "Centaur": 1.0, "Interstellar": 1.0,
           "Moon": 1.5, "Asteroid": 2.5, "default": 2.0}
SPECTRAL_DENSITY = {"C": 1.4, "B": 1.4, "Cb": 1.4, "D": 1.0, "Z": 1.0,
                    "S": 2.7, "Sk": 2.7, "Sq": 2.7, "X": 4.0, "Xe": 4.0, "M": 4.0}

COLOR = {
    "Star": (219, 222, 35), "Planet": (150, 150, 150), "Moon": (140, 140, 140),
    "Asteroid": (145, 107, 54), "TNO": (190, 185, 180), "Centaur": (160, 140, 130),
    "Comet": (170, 200, 210), "Interstellar": (200, 170, 210), "Dwarf Planet": (200, 195, 190),
}
# Colours already chosen in the repo, kept.
KEEP_COLOR = {
    "sol": (219,222,35), "mercury": (145,145,145), "venus": (224,224,224),
    "earth": (59,179,75), "mars": (242,66,17), "Jupiter": (201,144,57),
    "saturn": (206,184,124), "Uranus": (60,186,180), "Neptune": (60,186,180),
    "eris": (200,200,200), "dysnomia": (200,200,200), "Sedna": (200,200,200),
    "1-ceres": (145,107,54), "4-vesta": (145,107,54), "luna": (87,87,87),
}
# Exert Newtonian gravity / drive `mu` for something else.
MAJOR = {"sol","mercury","venus","earth","mars","Jupiter","saturn","Uranus","Neptune",
         "pluto","luna","io","europa","ganymede","callisto","titan","triton","charon",
         "1-ceres","eris","4-vesta","2-pallas"}

DESIGNATION = {"luna": "Earth I", "1-ceres": "1 Ceres", "4-vesta": "4 Vesta",
               "eris": "136199 Eris", "dysnomia": "Eris I", "Sedna": "90377 Sedna"}

def radius_m(slug, v):
    if slug in EXISTING_DIAMETER_KM:
        return EXISTING_DIAMETER_KM[slug] * 500.0, "repo value, halved (it was a diameter)"
    if v["radius_km"]:
        return v["radius_km"] * 1000.0, None
    # D(km) = 1329 / sqrt(albedo) * 10^(-H/5) -- standard absolute-magnitude estimator.
    import re
    t = open(f"raw/phys_{slug}.txt").read()
    m = re.search(r"\bH=\s*([-+]?[\d.]+)", t)
    if not m: return None, None
    h = float(m.group(1))
    albedo = 0.09
    d = 1329.0 / math.sqrt(albedo) * 10 ** (-h / 5.0)
    return d * 500.0, f"ESTIMATE from H={h} at assumed albedo {albedo}"

def mass_kg(slug, v, r_m):
    if v["mass_kg"]: return v["mass_kg"], None
    if slug in EXISTING_MASS_KG:
        return EXISTING_MASS_KG[slug], "repo value; JPL publishes no GM for this body"
    if not r_m: return 0.0, "unknown; no GM and no size published"
    styp = v.get("spectral_type")
    rho = SPECTRAL_DENSITY.get(styp) if styp else None
    if rho is None:
        rho = next((DENSITY[t] for t in v["tags"] if t in DENSITY), DENSITY["default"])
    vol = 4.0 / 3.0 * math.pi * r_m ** 3
    return vol * rho * 1000.0, f"ESTIMATE from radius at assumed density {rho} g/cm3"

def color(slug, tags):
    if slug in KEEP_COLOR: return KEEP_COLOR[slug]
    for t in tags:
        if t in COLOR: return COLOR[t]
    return (150, 150, 150)

def fmt(x, digits=10):
    """Always a Rust f64 literal: `%g` can emit `0` or `24397000`, which parse as ints."""
    out = f"{x:.{digits}g}"
    if "." not in out and "e" not in out and "E" not in out and "inf" not in out:
        out += ".0"
    return out

def emit(slug, name, cmd, prim, tags, center):
    v = phys[slug]; f = fits[slug]
    n_deg_day, a_m, e, inc, raan, argp, m0, argp_rate, raan_rate = f["params"]
    r_m, r_note = radius_m(slug, v)
    m_kg, m_note = mass_kg(slug, v, r_m)
    rgb = color(slug, tags)
    period_days = 360.0 / n_deg_day

    precessing = abs(argp_rate) > 1e-9 or abs(raan_rate) > 1e-9
    if precessing:
        rot = (f"KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {{\n"
               f"                            inclination: {fmt(inc)},\n"
               f"                            longitude_of_ascending_node: {fmt(raan % 360.0)},\n"
               f"                            argument_of_periapsis: {fmt(argp % 360.0)},\n"
               f"                            apsidal_precession_period: TimeDelta::from_days({fmt(360.0/argp_rate) if abs(argp_rate)>1e-12 else '1e18'}),\n"
               f"                            nodal_precession_period: TimeDelta::from_days({fmt(360.0/raan_rate) if abs(raan_rate)>1e-12 else '1e18'}),\n"
               f"                        }})")
    else:
        rot = (f"KeplerRotation::EulerAngles(KeplerEulerAngles {{\n"
               f"                            inclination: {fmt(inc)},\n"
               f"                            longitude_of_ascending_node: {fmt(raan % 360.0)},\n"
               f"                            argument_of_periapsis: {fmt(argp % 360.0)},\n"
               f"                        }})")

    notes = [f"// {name}: fitted to {f['n_samples']} JPL states over {f['span_yr']:.1f} yr; "
             f"residual {f['rms_m']/1e3:.0f} km RMS ({f['rel_rms']:.1e} of orbit radius)."]
    if f["rel_rms"] > 0.05:
        notes.append("// A two-body model is a poor fit here; treat its position as indicative.")
    if slug in BARYCENTRIC:
        notes.append("// Orbits the Pluto-Charon barycentre; modelled about Pluto, so it carries")
        notes.append("// a further ~2100 km offset the fit cannot remove.")
    if r_note: notes.append(f"// radius: {r_note}")
    if m_note: notes.append(f"// mass: {m_note}")
    note_block = "\n                ".join(notes)

    desig = DESIGNATION.get(slug)
    desig_s = f'Some("{desig}".into())' if desig else "None"
    appearance = (f"Appearance::DebugBall(DebugBall {{\n"
                  f"                        radius: {fmt(r_m or 1000.0)},\n"
                  f"                        color: AppearanceColor {{ r: {rgb[0]}, g: {rgb[1]}, b: {rgb[2]} }},\n                        highlight_latitudes: Vec::new(),\n"
                  f"                    }})")
    return f"""                {note_block}
                SomeBody::KeplerEntry(KeplerEntry {{
                    info: BodyInfo {{
                        name: Some("{name}".into()),
                        id: "{slug}".to_string(),
                        mass: {fmt(m_kg)},
                        major: {"true" if slug in MAJOR else "false"},
                        designation: {desig_s},
                        tags: vec![{", ".join(f'"{t}".into()' for t in tags)}],
                        ..Default::default()
                    }},
                    params: KeplerMotive {{
                        primary_id: "{prim}".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {{
                            eccentricity: {fmt(e)},
                            semi_major_axis: {fmt(a_m)},
                        }}),
                        rotation: {rot},
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {{
                            mean_anomaly: {fmt(m0 % 360.0)},
                        }}),
                        anomalistic_period: Some(TimeDelta::from_days({fmt(period_days)})),
                    }},
                    appearance: {appearance},
                    rotation: None,
                }}),"""

if __name__ == "__main__":
    parts = []
    for slug, name, cmd, prim, tags, center in all_bodies():
        if slug == "sol": continue
        if slug not in fits: print("SKIP", slug); continue
        parts.append(emit(slug, name, cmd, prim, tags, center))
    open("generated_bodies.rs","w").write("\n".join(parts))
    est_r = [s for s in fits if radius_m(s, phys[s])[1] and "ESTIMATE" in (radius_m(s, phys[s])[1] or "")]
    print(f"emitted {len(parts)} bodies")
    print(f"estimated radii ({len(est_r)}):", " ".join(est_r))
