"""Fetch elements + position series and fit, for every body in the user's save that the
first pass did not already cover. Reuses the same seeding/continuation/bounding logic."""
import json, math, os, subprocess, urllib.parse
from multiprocessing import Pool
import fitlib as F
from merged_table import save, resolved, ALIAS

DAY, J2000, N = 86400.0, 2451545.0, 1500
os.makedirs("raw2", exist_ok=True); os.makedirs("series2", exist_ok=True)
have = {v: k for k, v in ALIAS.items()}          # save-id -> my slug, where it already exists
COLS = "JDTDB CAL EC QR IN OM W Tp N MA TA A AD PR".split()

def horizons(p):
    return subprocess.run(["curl","-sS","--max-time","180",
        f"https://ssd.jpl.nasa.gov/api/horizons.api?{urllib.parse.urlencode(p)}"],
        capture_output=True, text=True).stdout

def elements(cmd, center):
    t = horizons({"format":"text","COMMAND":f"'{cmd}'","OBJ_DATA":"NO","MAKE_EPHEM":"YES",
        "EPHEM_TYPE":"ELEMENTS","CENTER":f"'{center}'","REF_PLANE":"ECLIPTIC",
        "REF_SYSTEM":"ICRF","OUT_UNITS":"KM-S","CSV_FORMAT":"YES",
        "TLIST":"2451545.0","TLIST_TYPE":"JD"})
    inside=False
    for line in t.splitlines():
        if line.startswith("$$SOE"): inside=True; continue
        if line.startswith("$$EOE"): break
        if inside:
            p=[x.strip() for x in line.split(",")]
            if len(p)>=14:
                d={}
                for i,n in enumerate(COLS):
                    if n=="CAL": continue
                    try: d[n]=float(p[i])
                    except ValueError: pass
                return d
    return None

def series(cmd, center, e):
    ec=e.get("EC",0.0)
    if ec>=1.0 or e.get("A",1)<0:
        tp=e.get("Tp",J2000); start,stop=tp-5*365.25, tp+5*365.25
    else:
        p=e.get("PR",0.0)/DAY or 365.25
        start=J2000; stop=J2000+min(max(250.0*p,60.0),60*365.25)
    step=max((stop-start)/N, 1/1440)
    t = horizons({"format":"text","COMMAND":f"'{cmd}'","OBJ_DATA":"NO","MAKE_EPHEM":"YES",
        "EPHEM_TYPE":"VECTORS","CENTER":f"'{center}'","REF_PLANE":"ECLIPTIC","REF_SYSTEM":"ICRF",
        "VEC_TABLE":"2","VEC_CORR":"NONE","OUT_UNITS":"KM-S","CSV_FORMAT":"YES",
        "START_TIME":f"'JD{start:.6f}'","STOP_TIME":f"'JD{stop:.6f}'",
        "STEP_SIZE":f"'{step*24*60:.0f}m'"})
    rows=[];inside=False
    for line in t.splitlines():
        if line.startswith("$$SOE"): inside=True; continue
        if line.startswith("$$EOE"): break
        if inside:
            p=[x.strip() for x in line.split(",")]
            if len(p)>=8:
                try: rows.append([float(p[0])]+[float(p[i]) for i in (2,3,4,5,6,7)])
                except ValueError: pass
    return {"start":start,"stop":stop,"rows":rows}

STALE = json.load(open("stale.json"))
todo = [b for b in save if (b not in have or b in STALE) and "error" not in resolved.get(b, {"error":1})
        and save[b]["kind"] == "kepler"]
print(f"{len(todo)} bodies to fetch", flush=True)
for bid in todo:
    ep=f"raw2/elem_{bid}.json"; sp=f"series2/{bid}.json"
    if os.path.exists(sp) and len(json.load(open(sp)).get("rows",[]))>50: continue
    r=resolved[bid]
    e = json.load(open(ep)) if os.path.exists(ep) else elements(r["command"], r["center"])
    if not e: print(f"  {bid}: no elements", flush=True); continue
    json.dump(e, open(ep,"w"))
    s = series(r["command"], r["center"], e)
    if len(s["rows"])<50: print(f"  {bid}: {len(s['rows'])} rows", flush=True); continue
    json.dump(s, open(sp,"w"))
print("fetch done", flush=True)

def seed(bid):
    e=json.load(open(f"raw2/elem_{bid}.json"))
    return [e["N"]*DAY, e["A"]*1000.0, e["EC"], e["IN"], e["OM"], e["W"], e["MA"]%360.0, 0.0, 0.0]

def fit_one(bid):
    try:
        s=json.load(open(f"series2/{bid}.json"))
        data=[((r[0]-J2000)*DAY, r[1]*1e3, r[2]*1e3, r[3]*1e3) for r in s["rows"]]
        p0=seed(bid); base,_=F.rms_max(p0,data)
        bounds={1:(0.5*p0[1],2.0*p0[1]) if p0[1]>0 else (2.0*p0[1],0.5*p0[1]),
                0:(0.85*p0[0],1.15*p0[0]) if p0[0]>0 else (1.15*p0[0],0.85*p0[0])}
        span=(data[-1][0]-data[0][0])/DAY; per=abs(360.0/p0[0]) if p0[0] else span
        first=min(1.0,max(60.0/len(data), 5.0*per/max(span,1e-9)))
        def run(p,free,f0):
            frac=f0
            while True:
                sub=data[:max(60,int(len(data)*min(frac,1.0)))]
                p=F.gauss_newton(p, sub[::max(1,len(sub)//400)], free, bounds=bounds)
                if frac>=1.0: break
                frac=min(frac*3.0,1.0) if frac*3.0<1.0 else 1.0
            return p
        starts=sorted({first,min(1.0,first*4),min(1.0,first*16),0.12,0.4})
        best=None
        for f0 in starts:
            c=run(list(p0),[0,1,2,3,4,5,6],f0); r,m=F.rms_max(c,data)
            if best is None or r<best[0]: best=(r,m,c)
        r7,m7,p7=best
        r9,m9,p9=r7,m7,p7
        for f0 in starts:
            c=run(list(p7),[0,1,2,3,4,5,6,7,8],f0); r,m=F.rms_max(c,data)
            if r<r9: r9,m9,p9=r,m,c
        if r9<r7*0.9: p,rms,mx,kind=p9,r9,m9,"precessing"
        else: p,rms,mx,kind=p7,r7,m7,"fixed"
        scale=sum(math.dist((0,0,0),(d[1],d[2],d[3])) for d in data)/len(data)
        return bid,{"params":p,"rms_m":rms,"max_m":mx,"kind":kind,
                    "rel_rms":rms/scale if scale else float("inf"),"baseline_rms_m":base,
                    "n_samples":len(data),"span_yr":(s["stop"]-s["start"])/365.25},None
    except Exception as ex:
        return bid,None,f"{type(ex).__name__}: {ex}"

if __name__=="__main__":
    fit_targets=[b for b in todo if os.path.exists(f"series2/{b}.json")]
    print(f"fitting {len(fit_targets)}", flush=True)
    out={}
    with Pool(8) as pool:
        for bid,res,err in pool.imap_unordered(fit_one, fit_targets):
            if err: print(f"  {bid:20s} SKIP {err}", flush=True); continue
            out[bid]=res
            print(f"  {bid:20s} {res['kind']:10s} rel {res['rel_rms']:.2e}", flush=True)
    json.dump(out, open("fits2.json","w"), indent=1)
    print(f"fitted {len(out)}")
