import json, re
from bodies import all_bodies
G = 6.6743015e-11
NUM = r"([-+]?\d*\.?\d+(?:[eEdD][-+]?\d+)?)"

def f(s):
    try: return float(s.replace("D","e").replace("d","e"))
    except Exception: return None

def parse(txt):
    gm = rad = rot = styp = None
    # --- GM (km^3/s^2) in any of the layouts Horizons uses ---
    for pat in [rf"GM\s*\(km\^3/s\^2\)\s*=\s*{NUM}",
                rf"GM\s*\(planet\)\s*km\^3/s\^2\s*=\s*{NUM}",
                rf"GM\s*,?\s*km\^3/s\^2\s*=\s*{NUM}",
                rf"GM=\s*{NUM}",
                rf"GM\s*\(km\^3/s\^2\)\s*,?\s*=\s*{NUM}"]:
        m = re.search(pat, txt)
        if m and f(m.group(1)) not in (None, 0.0):
            gm = f(m.group(1)); break
    # Mass x 10^N (kg) fallback (gas giants list mass, not always GM)
    if gm is None:
        m = re.search(rf"Mass\s*x?\s*10\^(\d+)\s*\(?kg\)?\s*=?\s*~?\s*{NUM}", txt)
        if m:
            mass = f(m.group(2)) * 10 ** int(m.group(1))
            gm = mass * G / 1e9

    # --- radius: triaxial "a x b x c" first, then scalar forms ---
    m = re.search(rf"[Rr]adius[^=\n]*=\s*{NUM}\s*x\s*{NUM}\s*x\s*{NUM}", txt)
    if m:
        a, b, c = f(m.group(1)), f(m.group(2)), f(m.group(3))
        if a and b and c: rad = (a * b * c) ** (1 / 3)
    if rad is None:
        for pat in [rf"RAD=\s*{NUM}",
                    rf"Vol\. [Mm]ean [Rr]adius[^=\n]*=\s*{NUM}",
                    rf"Mean [Rr]adius\s*\(km\)[^=\n]*=\s*{NUM}",
                    rf"Volumetric mean radius[^=\n]*=\s*{NUM}",
                    rf"Radius\s*\(km[^)]*\)\s*=\s*{NUM}",
                    rf"Equat\. radius[^=\n]*=\s*{NUM}",
                    rf"[Rr]adius[^=\n]*\(km\)[^=\n]*=\s*{NUM}"]:
            m = re.search(pat, txt)
            if m and f(m.group(1)): rad = f(m.group(1)); break

    for pat in [rf"ROTPER=\s*{NUM}", rf"Sid\. rot\. period[^=\n]*=\s*{NUM}\s*h",
                rf"Sidereal rot\. period[^=\n]*=\s*{NUM}\s*h",
                rf"Rotational period[^=\n]*=\s*{NUM}\s*h"]:
        m = re.search(pat, txt)
        if m and f(m.group(1)): rot = f(m.group(1)); break

    m = re.search(r"STYP=\s*(\S+)", txt)
    if m and m.group(1) != "n.a.": styp = m.group(1)
    return gm, rad, rot, styp

out = {}
for slug, name, cmd, prim, tags, center in all_bodies():
    txt = open(f"raw/phys_{slug}.txt").read()
    gm, rad, rot, styp = parse(txt)
    out[slug] = {"name": name, "command": cmd, "primary": prim, "tags": tags,
                 "center": center, "gm_km3": gm, "radius_km": rad,
                 "rot_period_h": rot, "spectral_type": styp,
                 "mass_kg": gm * 1e9 / G if gm else None}
json.dump(out, open("phys.json","w"), indent=1)
nm = [s for s,v in out.items() if not v["mass_kg"]]
nr = [s for s,v in out.items() if not v["radius_km"]]
print(f"missing mass: {len(nm)}   missing radius: {len(nr)}")
print("no mass:  ", " ".join(nm))
print("no radius:", " ".join(nr))
