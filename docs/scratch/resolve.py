import json, os, re, subprocess, urllib.parse
save = json.load(open("user_save.json"))
os.makedirs("raw2", exist_ok=True)

CENTER = {"Sol":"500@10","Earth":"500@399","Mars":"500@499","Jupiter":"500@599",
          "Saturn":"500@699","Uranus":"500@799","Neptune":"500@899",
          "Pluto Barycenter":"500@9","Eris":"500@20136199"}
# Small bodies need the ';' small-body form; a couple resolve better by number.
EXPLICIT = {"Sol":"10","Pluto Barycenter":"9","1-Ceres":"1;","4-Vesta":"4;",
            "Eris":"136199;","Sedna":"90377;","Pluto":"999","Luna":"301"}

def variants(name):
    yield name
    m = re.fullmatch(r"S(\d{4}) ([A-Z]) ?(\d+)", name)      # S2003 J18 -> S/2003 J 18
    if m: yield f"S/{m.group(1)} {m.group(2)} {m.group(3)}"
    if " " in name: yield name.replace(" ", "")

def probe(cmd, center):
    q = urllib.parse.urlencode({"format":"text","COMMAND":f"'{cmd}'","OBJ_DATA":"NO",
        "MAKE_EPHEM":"YES","EPHEM_TYPE":"ELEMENTS","CENTER":f"'{center}'",
        "REF_PLANE":"ECLIPTIC","OUT_UNITS":"KM-S","CSV_FORMAT":"YES",
        "TLIST":"2451545.0","TLIST_TYPE":"JD"})
    t = subprocess.run(["curl","-sS","--max-time","60",
        f"https://ssd.jpl.nasa.gov/api/horizons.api?{q}"], capture_output=True, text=True).stdout
    m = re.search(r"Target body name: (.+?)\s*\{", t)
    ok = "$$SOE" in t
    return (m.group(1).strip() if m else None), ok

out = {}
for bid, v in sorted(save.items()):
    if v["kind"] == "fixed":
        out[bid] = {"command": "10", "center": None, "resolved": "Sun"}
        continue
    center = CENTER.get(v["primary"])
    if center is None:
        out[bid] = {"error": f"no center for primary {v['primary']!r}"}; continue
    cached = f"raw2/resolve_{bid.replace('/','_')}.json"
    if os.path.exists(cached):
        out[bid] = json.load(open(cached)); continue
    got = None
    for cand in ([EXPLICIT[bid]] if bid in EXPLICIT else list(variants(v["name"] or bid))):
        target, ok = probe(cand, center)
        if ok:
            got = {"command": cand, "center": center, "resolved": target}
            break
    out[bid] = got or {"error": "unresolved", "center": center}
    json.dump(out[bid], open(cached,"w"))
    print(f"{bid:22s} -> {out[bid].get('resolved') or out[bid]['error']}", flush=True)
json.dump(out, open("resolved.json","w"), indent=1)
bad = [k for k,v in out.items() if "error" in v]
print(f"\nresolved {len(out)-len(bad)}/{len(out)}; unresolved: {bad}")
