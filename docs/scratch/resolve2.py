import json, os, re, subprocess, urllib.parse
save = json.load(open("user_save.json"))
resolved = json.load(open("resolved.json"))
CENTER = {"Sol":"500@10","Earth":"500@399","Mars":"500@499","Jupiter":"500@599",
          "Saturn":"500@699","Uranus":"500@799","Neptune":"500@899",
          "Pluto Barycenter":"500@9","Eris":"500@20136199"}

def call(cmd, center):
    q = urllib.parse.urlencode({"format":"text","COMMAND":f"'{cmd}'","OBJ_DATA":"NO",
        "MAKE_EPHEM":"YES","EPHEM_TYPE":"ELEMENTS","CENTER":f"'{center}'",
        "REF_PLANE":"ECLIPTIC","OUT_UNITS":"KM-S","CSV_FORMAT":"YES",
        "TLIST":"2451545.0","TLIST_TYPE":"JD"})
    return subprocess.run(["curl","-sS","--max-time","60",
        f"https://ssd.jpl.nasa.gov/api/horizons.api?{q}"], capture_output=True, text=True).stdout

def pick_from_table(txt, want):
    """Horizons answers an ambiguous name with a table of ID# / Name. Take the exact match."""
    rows = re.findall(r"^\s*(-?\d+)\s\s+(\S.*?)\s{2,}", txt, re.M)
    want_n = want.lower().replace(" ", "").replace("/", "")
    exact = [i for i, n in rows if n.strip().lower().replace(" ", "").replace("/", "") == want_n]
    return exact[0] if exact else None

def try_name(name, center):
    txt = call(name, center)
    if "$$SOE" in txt:
        m = re.search(r"Target body name: (.+?)\s*\{", txt)
        return name, (m.group(1).strip() if m else name)
    pick = pick_from_table(txt, name)
    if pick:
        t2 = call(pick, center)
        if "$$SOE" in t2:
            m = re.search(r"Target body name: (.+?)\s*\{", t2)
            return pick, (m.group(1).strip() if m else pick)
    return None, None

bad = [k for k, v in resolved.items() if "error" in v]
print(f"retrying {len(bad)}", flush=True)
for bid in bad:
    v = save[bid]
    center = CENTER.get(v["primary"])
    name = v["name"] or bid
    cands = [name]
    m = re.fullmatch(r"S(\d{4}) ([A-Z]) ?(\d+)", name)
    if m: cands += [f"S/{m.group(1)} {m.group(2)} {m.group(3)}", f"{m.group(1)} {m.group(2)}{m.group(3)}"]
    got = None
    for c in cands:
        cmd, target = try_name(c, center)
        if cmd: got = {"command": cmd, "center": center, "resolved": target}; break
    resolved[bid] = got or {"error": "unresolved", "center": center}
    print(f"  {bid:20s} -> {resolved[bid].get('resolved') or 'UNRESOLVED'}", flush=True)
json.dump(resolved, open("resolved.json","w"), indent=1)
still = [k for k,v in resolved.items() if "error" in v]
print(f"\nnow resolved {len(resolved)-len(still)}/{len(resolved)}; still missing: {still}")
