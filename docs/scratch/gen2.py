"""Emit the merged Solar System.

The user's save is authoritative for identity, mass, radius, colour, tags and rotation.
JPL supplies the orbital elements. Bodies JPL cannot supply keep the save's elements.
"""
import json, math, sqlite3
from merged_table import save, resolved, ALIAS, EXTRA_IDS, PRIMARY_OF_EXTRA

DB = "/Users/zandy/Downloads/solar_system.em"
G = 6.6743015e-11
f1 = json.load(open("fits.json")); f2 = json.load(open("fits2.json"))
phys = json.load(open("phys.json"))
inv = {v: k for k, v in ALIAS.items()}
c = sqlite3.connect(DB)

app = {r[0]: dict(zip([d[1] for d in c.execute("pragma table_info(appearances)")], r))
       for r in c.execute("select * from appearances")}
rotcols = [d[1] for d in c.execute("pragma table_info(body_rotations)")]
rot = {r[0]: dict(zip(rotcols, r)) for r in c.execute("select * from body_rotations")}
kepcols = [d[1] for d in c.execute("pragma table_info(motive_keplerian)")]
userkep = {}
for r in c.execute("""select m.body_id, k.* from motives m join motive_keplerian k on k.motive_id=m.id"""):
    userkep[r[0]] = dict(zip(["body_id"] + kepcols, r))

# Stored as diameters under a field named `radius`.
DIAMETER_BUG = {"Eris", "Sedna"}
EXTRA_TAGS = {"Moon": ["Moon", "Minor Moon"]}
BIG_MOONS = {"Triton","Titania","Oberon","Ariel","Umbriel","Miranda","Proteus","Nereid"}

def fmt(x, d=10):
    s = f"{x:.{d}g}"
    return s if ("." in s or "e" in s or "E" in s) else s + ".0"

def fit_for(bid):
    if bid in f2: return f2[bid]
    if bid in inv and inv[bid] in f1: return f1[inv[bid]]
    slug = next((k for k, v in EXTRA_IDS.items() if v == bid), None)
    return f1.get(slug) if slug else None

def appearance_of(bid):
    a = app.get(bid)
    if a:
        r = a["radius"] or 1000.0
        # A barycentre has no body to draw, so its appearance row is mostly null.
        cr, cg, cb = (a["color_r"] if a["color_r"] is not None else 120,
                      a["color_g"] if a["color_g"] is not None else 120,
                      a["color_b"] if a["color_b"] is not None else 120)
        lr, lg, lb = (a["light_r"] or 0, a["light_g"] or 0, a["light_b"] or 0)
        am = a["absolute_magnitude"] if a["absolute_magnitude"] is not None else 4.83
        if bid in DIAMETER_BUG: r /= 2.0
        if a["appearance_type"] == "Star":
            return (f"""Appearance::Star(StarBall {{
                        radius: {fmt(r)},
                        color: AppearanceColor {{ r: {cr}, g: {cg}, b: {cb} }},
                        light: AppearanceColor {{ r: {lr}, g: {lg}, b: {lb} }},
                        absolute_magnitude: {am},
                    }})""", r, "halved: the save stored a diameter" if bid in DIAMETER_BUG else None)
        return (f"""Appearance::DebugBall(DebugBall {{
                        radius: {fmt(r)},
                        color: AppearanceColor {{ r: {cr}, g: {cg}, b: {cb} }},
                    }})""", r, "halved: the save stored a diameter" if bid in DIAMETER_BUG else None)
    # extras: fall back to the JPL-derived values from the first pass
    slug = next((k for k, v in EXTRA_IDS.items() if v == bid), None)
    v = phys.get(slug, {})
    r = (v.get("radius_km") or 1.0) * 1000.0
    note = None
    if not v.get("radius_km"):
        import re
        t = open(f"raw/phys_{slug}.txt").read()
        m = re.search(r"\bH=\s*([-+]?[\d.]+)", t)
        if m:
            r = 1329.0 / math.sqrt(0.09) * 10 ** (-float(m.group(1)) / 5.0) * 500.0
            note = f"ESTIMATE from H={m.group(1)} at assumed albedo 0.09"
    return (f"""Appearance::DebugBall(DebugBall {{
                        radius: {fmt(r)},
                        color: AppearanceColor {{ r: 170, g: 165, b: 160 }},
                    }})""", r, note)

def rotation_of(bid):
    r = rot.get(bid)
    if not r: return "None"
    if r["mode"] == "TidallyLocked":
        return (f"""Some(BodyRotation::tidally_locked("{r['primary_id']}",
                        DVec3::new({fmt(r['pole_x'])}, {fmt(r['pole_y'])}, {fmt(r['pole_z'])})))""")
    ep = "RotationEpoch::J2000" if r["epoch_type"] == "J2000" else \
         f"RotationEpoch::JulianDay({fmt(r['epoch_julian_day'] or 2451545.0)})"
    return (f"""Some(BodyRotation::spinning(
                        DQuat::from_xyzw({fmt(r['orientation_x'])}, {fmt(r['orientation_y'])}, {fmt(r['orientation_z'])}, {fmt(r['orientation_w'])}),
                        {fmt(r['angular_velocity'])}, {ep}))""")

def mass_of(bid):
    if bid in save: return save[bid]["mass"], None
    slug = next((k for k, v in EXTRA_IDS.items() if v == bid), None)
    v = phys.get(slug, {})
    if v.get("mass_kg"): return v["mass_kg"], None
    _, r, _ = appearance_of(bid)
    rho = 1.5 if "Moon" in (v.get("tags") or []) else 2.0
    return 4/3 * math.pi * r**3 * rho * 1000.0, f"ESTIMATE from radius at assumed density {rho} g/cm3"

def tags_of(bid):
    if bid in save and save[bid]["tags"]: return save[bid]["tags"]
    slug = next((k for k, v in EXTRA_IDS.items() if v == bid), None)
    t = phys.get(slug, {}).get("tags", ["Minor Planet"])
    if "Moon" in t: return ["Moon", "Major Moon" if bid in BIG_MOONS else "Minor Moon"]
    return t

def emit(bid, primary):
    fit = fit_for(bid)
    ap, radius, rnote = appearance_of(bid)
    mass, mnote = mass_of(bid)
    tags = tags_of(bid)
    name = save[bid]["name"] if bid in save and save[bid]["name"] else bid
    desig = save[bid].get("designation") if bid in save else None
    notes = []

    if fit:
        n, a, e, inc, raan, argp, m0, ar, nr = fit["params"]
        km = fit["rms_m"] / 1e3
        km_s = f"{km:.3g}" if km < 100 else f"{km:.0f}"
        notes.append(f"// {name}: fitted to {fit['n_samples']} JPL states over "
                     f"{fit['span_yr']:.1f} yr; residual {km_s} km RMS "
                     f"({fit['rel_rms']:.1e} of orbit radius).")
        if fit["rel_rms"] > 0.05:
            notes.append("// A fixed ellipse fits this poorly; treat the position as indicative.")
        period = 360.0 / n
        anom = f"Some(TimeDelta::from_days({fmt(period)}))"
        if abs(ar) > 1e-9 or abs(nr) > 1e-9:
            rotexpr = (f"""KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {{
                            inclination: {fmt(inc)},
                            longitude_of_ascending_node: {fmt(raan % 360)},
                            argument_of_periapsis: {fmt(argp % 360)},
                            apsidal_precession_period: TimeDelta::from_days({fmt(360.0/ar) if abs(ar)>1e-12 else '1e18'}),
                            nodal_precession_period: TimeDelta::from_days({fmt(360.0/nr) if abs(nr)>1e-12 else '1e18'}),
                        }})""")
        else:
            rotexpr = (f"""KeplerRotation::EulerAngles(KeplerEulerAngles {{
                            inclination: {fmt(inc)},
                            longitude_of_ascending_node: {fmt(raan % 360)},
                            argument_of_periapsis: {fmt(argp % 360)},
                        }})""")
        shape = f"eccentricity: {fmt(e)},\n                            semi_major_axis: {fmt(a)},"
        epoch = f"mean_anomaly: {fmt(m0 % 360)},"
    else:
        k = userkep[bid]
        notes.append(f"// {name}: JPL publishes no ephemeris under this designation, so these")
        notes.append("// are the elements the save already carried, unmodified.")
        rotexpr = (f"""KeplerRotation::EulerAngles(KeplerEulerAngles {{
                            inclination: {fmt(k['inclination'] or 0.0)},
                            longitude_of_ascending_node: {fmt(k['longitude_of_ascending_node'] or 0.0)},
                            argument_of_periapsis: {fmt(k['argument_of_periapsis'] or 0.0)},
                        }})""")
        shape = (f"eccentricity: {fmt(k['eccentricity'] or 0.0)},\n"
                 f"                            semi_major_axis: {fmt(k['semi_major_axis'] or 1.0)},")
        epoch = f"mean_anomaly: {fmt((k['mean_anomaly'] or 0.0) % 360)},"
        anom = "None"

    if rnote: notes.append(f"// radius: {rnote}")
    if mnote: notes.append(f"// mass: {mnote}")
    nb = "\n                ".join(notes)
    dg = f'Some("{desig}".into())' if desig else "None"
    return f"""                {nb}
                SomeBody::KeplerEntry(KeplerEntry {{
                    info: BodyInfo {{
                        name: Some("{name}".into()),
                        id: "{bid}".to_string(),
                        mass: {fmt(mass)},
                        major: {"true" if (bid in save and save[bid]["major"]) else "false"},
                        designation: {dg},
                        tags: vec![{", ".join(f'"{t}".into()' for t in tags)}],
                        ..Default::default()
                    }},
                    params: KeplerMotive {{
                        primary_id: "{primary}".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {{
                            {shape}
                        }}),
                        rotation: {rotexpr},
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {{
                            {epoch}
                        }}),
                        anomalistic_period: {anom},
                    }},
                    appearance: {ap},
                    rotation: {rotation_of(bid)},
                }}),"""

if __name__ == "__main__":
    ap_sol, _, _ = appearance_of("Sol")
    parts = [f"""                SomeBody::FixedEntry(FixedEntry {{
                    info: BodyInfo {{
                        name: Some("Sol".into()),
                        id: "Sol".to_string(),
                        mass: {fmt(save['Sol']['mass'])},
                        major: true,
                        designation: None,
                        tags: vec![{", ".join(f'"{t}".into()' for t in save['Sol']['tags'])}],
                        ..Default::default()
                    }},
                    position: DVec3::ZERO,
                    appearance: {ap_sol},
                    rotation: {rotation_of("Sol")},
                }}), // Sun"""]
    order = [b for b in save if save[b]["kind"] == "kepler"] + list(EXTRA_IDS.values())
    for bid in order:
        prim = save[bid]["primary"] if bid in save else PRIMARY_OF_EXTRA.get(bid, "Sol")
        parts.append(emit(bid, prim))
    open("generated_bodies2.rs", "w").write("\n".join(parts))
    print(f"emitted {len(parts)} bodies (1 fixed + {len(parts)-1} keplerian)")
