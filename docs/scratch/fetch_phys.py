import json, os, re, subprocess, urllib.parse
from bodies import all_bodies

os.makedirs("raw", exist_ok=True)
G = 6.6743015e-11   # matches UniversePhysics::default()

def fetch(cmd):
    q = urllib.parse.urlencode({"format":"text","COMMAND":f"'{cmd}'","OBJ_DATA":"YES","MAKE_EPHEM":"NO"})
    return subprocess.run(["curl","-sS","--max-time","60",
        f"https://ssd.jpl.nasa.gov/api/horizons.api?{q}"],
        capture_output=True, text=True).stdout

def num(s):
    s = s.replace("D","e").strip()
    try: return float(s)
    except ValueError: return None

def parse(txt):
    gm_km3 = rad_km = rotper_h = None
    # small-body block
    m = re.search(r"GM=\s*([\d.eE+-]+)", txt)
    if m: gm_km3 = num(m.group(1))
    m = re.search(r"RAD=\s*([\d.eE+-]+)", txt)
    if m: rad_km = num(m.group(1))
    m = re.search(r"ROTPER=\s*([\d.eE+-]+)", txt)
    if m: rotper_h = num(m.group(1))
    # major-body block
    if gm_km3 is None:
        m = re.search(r"GM[^=\n]*\(km\^3/s\^2\)\s*=\s*([\d.eE+-]+)", txt) \
            or re.search(r"GM,? ?\(?km\^3/s\^2\)?\s*=\s*([\d.eE+-]+)", txt) \
            or re.search(r"GM \(km\^3/s\^2\)\s*=\s*([\d.eE+-]+)", txt)
        if m: gm_km3 = num(m.group(1))
    if rad_km is None:
        for pat in [r"Vol\. [Mm]ean [Rr]adius[^=\n]*=\s*([\d.]+)",
                    r"Mean radius \(km\)\s*=\s*([\d.]+)",
                    r"Volumetric mean radius[^=\n]*=\s*([\d.]+)",
                    r"Radius \(km\)\s*=\s*([\d.]+)",
                    r"Radius[^=\n]*\(km\)\s*=\s*([\d.]+)",
                    r"Equat\. radius[^=\n]*=\s*([\d.]+)"]:
            m = re.search(pat, txt)
            if m: rad_km = num(m.group(1)); break
    if rotper_h is None:
        m = re.search(r"Sid\. rot\. period[^=\n]*=\s*([\d.]+)\s*h", txt)
        if m: rotper_h = num(m.group(1))
        else:
            m = re.search(r"Sidereal rot\. period[^=\n]*=\s*([\d.]+)\s*h", txt)
            if m: rotper_h = num(m.group(1))
    return gm_km3, rad_km, rotper_h

out = {}
for slug, name, cmd, prim, tags, center in all_bodies():
    path = f"raw/phys_{slug}.txt"
    if os.path.exists(path) and os.path.getsize(path) > 200:
        txt = open(path).read()
    else:
        txt = fetch(cmd); open(path,"w").write(txt)
    gm_km3, rad_km, rotper_h = parse(txt)
    mass = gm_km3 * 1e9 / G if gm_km3 else None   # km^3/s^2 -> m^3/s^2 -> kg
    out[slug] = {"name": name, "command": cmd, "primary": prim, "tags": tags,
                 "center": center, "gm_km3": gm_km3, "radius_km": rad_km,
                 "rot_period_h": rotper_h, "mass_kg": mass}
json.dump(out, open("phys.json","w"), indent=1)

missing_mass = [s for s,v in out.items() if not v["mass_kg"]]
missing_rad  = [s for s,v in out.items() if not v["radius_km"]]
print(f"{len(out)} bodies; missing mass: {len(missing_mass)}, missing radius: {len(missing_rad)}")
print("no mass:  ", " ".join(missing_mass))
print("no radius:", " ".join(missing_rad))
