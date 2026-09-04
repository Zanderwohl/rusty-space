import json, os, subprocess, urllib.parse
from bodies import all_bodies
os.makedirs("raw", exist_ok=True)

def horizons(params):
    q = urllib.parse.urlencode(params)
    return subprocess.run(["curl","-sS","--max-time","90",
        f"https://ssd.jpl.nasa.gov/api/horizons.api?{q}"], capture_output=True, text=True).stdout

COLS = "JDTDB CAL EC QR IN OM W Tp N MA TA A AD PR".split()

def get_elements(cmd, center, jd):
    txt = horizons({"format":"text","COMMAND":f"'{cmd}'","OBJ_DATA":"NO","MAKE_EPHEM":"YES",
        "EPHEM_TYPE":"ELEMENTS","CENTER":f"'{center}'","REF_PLANE":"ECLIPTIC",
        "REF_SYSTEM":"ICRF","OUT_UNITS":"KM-S","CSV_FORMAT":"YES",
        "TLIST":str(jd),"TLIST_TYPE":"JD"})
    inside = False
    for line in txt.splitlines():
        if line.startswith("$$SOE"): inside = True; continue
        if line.startswith("$$EOE"): break
        if inside:
            p = [x.strip() for x in line.split(",")]
            if len(p) < 14: continue
            vals = {}
            for i, name in enumerate(COLS):
                if name == "CAL": continue
                try: vals[name] = float(p[i])
                except ValueError: pass
            return vals
    return {"error": txt.strip().splitlines()[-1][:120] if txt.strip() else "empty"}

out = {}
for slug, name, cmd, prim, tags, center in all_bodies():
    if slug == "sol":
        continue
    path = f"raw/elem_{slug}.json"
    if os.path.exists(path):
        out[slug] = json.load(open(path)); continue
    e = get_elements(cmd, center, 2451545.0)
    if "error" in e:   # try a modern epoch: recent discoveries, hyperbolic visitors
        e2 = get_elements(cmd, center, 2460676.5)
        if "error" not in e2:
            e2["epoch_jd"] = 2460676.5; e = e2
    else:
        e["epoch_jd"] = 2451545.0
    out[slug] = e
    json.dump(e, open(path,"w"))
    print(f"{slug:22s} " + ("FAIL: " + e["error"] if "error" in e
          else f"a={e.get('A',0)/1e6:10.4f}e6 km  e={e.get('EC',0):.4f}  P={e.get('PR',0)/86400 if e.get('PR') else float('nan'):10.3f} d"), flush=True)
json.dump(out, open("elements.json","w"), indent=1)
print("\nfailures:", [s for s,v in out.items() if "error" in v])
