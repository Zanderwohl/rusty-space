"""Fetch a position series per body, sized from its own orbital period.

span = clamp(250 * period, 60 d, 60 yr) at 1500 samples, so a short-period body gets
~6 samples per orbit across many orbits (enough to constrain precession), while a
long-period one gets a densely sampled arc.
"""
import json, os, subprocess, urllib.parse
from bodies import all_bodies

os.makedirs("series", exist_ok=True)
DAY = 86400.0
J2000 = 2451545.0
elements = json.load(open("elements.json"))
N = 1500

def plan(slug, e):
    ec = e.get("EC", 0.0)
    if ec >= 1.0 or e.get("A", 1) < 0:           # hyperbolic: bracket perihelion
        tp = e.get("Tp", J2000)
        return tp - 5 * 365.25, tp + 5 * 365.25
    p_days = e.get("PR", 0.0) / DAY if e.get("PR") else 365.25
    span = min(max(250.0 * p_days, 60.0), 60 * 365.25)
    return J2000, J2000 + span

def fetch(cmd, center, start, stop, step_days):
    q = urllib.parse.urlencode({
        "format":"text","COMMAND":f"'{cmd}'","OBJ_DATA":"NO","MAKE_EPHEM":"YES",
        "EPHEM_TYPE":"VECTORS","CENTER":f"'{center}'","REF_PLANE":"ECLIPTIC",
        "REF_SYSTEM":"ICRF","VEC_TABLE":"2","VEC_CORR":"NONE","OUT_UNITS":"KM-S",
        "CSV_FORMAT":"YES","START_TIME":f"'JD{start:.6f}'","STOP_TIME":f"'JD{stop:.6f}'",
        "STEP_SIZE":f"'{step_days*24*60:.0f}m'"})
    return subprocess.run(["curl","-sS","--max-time","180",
        f"https://ssd.jpl.nasa.gov/api/horizons.api?{q}"], capture_output=True, text=True).stdout

def rows(txt):
    out, inside = [], False
    for line in txt.splitlines():
        if line.startswith("$$SOE"): inside = True; continue
        if line.startswith("$$EOE"): break
        if inside:
            p = [x.strip() for x in line.split(",")]
            if len(p) >= 8:
                try: out.append([float(p[0])] + [float(p[i]) for i in (2,3,4,5,6,7)])
                except ValueError: pass
    return out

done = fail = skip = 0
for slug, name, cmd, prim, tags, center in all_bodies():
    if slug == "sol": continue
    path = f"series/{slug}.json"
    if os.path.exists(path) and len(json.load(open(path)).get("rows", [])) > 50:
        skip += 1; continue
    e = elements.get(slug, {})
    start, stop = plan(slug, e)
    step = max((stop - start) / N, 1.0 / 1440.0)
    r = rows(fetch(cmd, center, start, stop, step))
    if len(r) < 50:
        print(f"FAIL {slug}: {len(r)} rows", flush=True); fail += 1; continue
    json.dump({"slug": slug, "center": center, "start": start, "stop": stop,
               "step_days": step, "rows": r}, open(path, "w"))
    done += 1
    print(f"{slug:22s} {len(r):5d} rows  span {(stop-start)/365.25:7.2f} yr  step {step:9.4f} d", flush=True)
print(f"\nfetched {done}, cached {skip}, failed {fail}")
