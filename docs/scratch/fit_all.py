import json, math, os, sys
from multiprocessing import Pool
import fitlib as F
from bodies import all_bodies

G = 6.6743015e-11
DAY = 86400.0
J2000 = 2451545.0
phys = json.load(open("phys.json"))
elements = json.load(open("elements.json"))

def mu_for(slug):
    prim = phys[slug]["primary"]
    mp = phys[prim]["mass_kg"] or 0.0
    mb = phys[slug]["mass_kg"] or 0.0
    return G * (mp + mb)

def seed(slug):
    e = elements[slug]
    epoch = e.get("epoch_jd", J2000)
    a_m = e["A"] * 1000.0
    ec = e["EC"]
    # N is deg/sec in Horizons ELEMENTS output; convert to deg/day.
    ndeg = e["N"] * DAY
    m0 = e["MA"] - ndeg * (epoch - J2000)      # shift the mean anomaly back to J2000
    return [ndeg, a_m, ec, e["IN"], e["OM"], e["W"], m0 % 360.0, 0.0, 0.0]

def fit_one(slug):
  try:
    try:
        s = json.load(open(f"series/{slug}.json"))
    except FileNotFoundError:
        return slug, None, "no series"
    data = [((r[0] - J2000) * DAY, r[1] * 1e3, r[2] * 1e3, r[3] * 1e3) for r in s["rows"]]
    p0 = seed(slug)

    base_rms, base_max = F.rms_max(p0, data)

    # Span continuation: short arcs first, where the mean anomaly is unambiguous, then
    # widen geometrically.
    #
    # The first window has to be only a few orbits. Mean motion from osculating elements
    # can be off by a percent or more for a close-in moon (Saturn's J2 on Pan is ~1.5%),
    # and across hundreds of orbits that is several revolutions of phase error — the
    # residual goes uncorrelated and there is no gradient to follow. Sizing the first
    # window at ~5 orbits keeps the phase unambiguous, and each later stage inherits a
    # mean motion good enough for the next.
    span_days = (data[-1][0] - data[0][0]) / DAY
    period_days = abs(360.0 / p0[0]) if p0[0] else span_days
    first = min(1.0, max(60.0 / len(data), 5.0 * period_days / max(span_days, 1e-9)))

    # `a` is well determined by the osculating elements; the mean motion is not, since
    # J2 and third-body effects shift it by up to a percent or two. Bound both.
    bounds = {1: (0.5 * p0[1], 2.0 * p0[1]) if p0[1] > 0 else (2.0 * p0[1], 0.5 * p0[1]),
              0: (0.85 * p0[0], 1.15 * p0[0]) if p0[0] > 0 else (1.15 * p0[0], 0.85 * p0[0])}

    def run(p, free, first_frac):
        frac = first_frac
        while True:
            sub = data[: max(60, int(len(data) * min(frac, 1.0)))]
            step = max(1, len(sub) // 400)
            p = F.gauss_newton(p, sub[::step], free, bounds=bounds)
            if frac >= 1.0: break
            frac = min(frac * 3.0, 1.0) if frac * 3.0 < 1.0 else 1.0
        return p

    # The right opening window is body-dependent — too wide and the phase is ambiguous,
    # too narrow and the arc does not constrain the shape. Try several and keep the best.
    starts = sorted({first, min(1.0, first * 4), min(1.0, first * 16), 0.12, 0.4})
    p7, r7, m7 = None, float("inf"), 0.0
    for f0 in starts:
        c = run(list(p0), [0, 1, 2, 3, 4, 5, 6], f0)
        r, m = F.rms_max(c, data)
        if r < r7: p7, r7, m7 = c, r, m

    p9, r9, m9 = p7, r7, m7
    for f0 in starts:
        c = run(list(p7), [0, 1, 2, 3, 4, 5, 6, 7, 8], f0)
        r, m = F.rms_max(c, data)
        if r < r9: p9, r9, m9 = c, r, m

    # Only pay for precession if it earns its place.
    if r9 < r7 * 0.9:
        best, rms, mx, kind = p9, r9, m9, "precessing"
    else:
        best, rms, mx, kind = p7, r7, m7, "fixed"

    scale = sum(math.dist((0,0,0), (d[1], d[2], d[3])) for d in data) / len(data)
    return slug, {"params": best, "rms_m": rms, "max_m": mx, "kind": kind,
                  "rel_rms": rms / scale if scale else float("inf"), "mean_radius_m": scale,
                  "baseline_rms_m": base_rms, "n_samples": len(data),
                  "span_yr": (s["stop"] - s["start"]) / 365.25}, None
  except Exception as ex:
    import traceback
    return slug, None, f"{type(ex).__name__}: {ex} | {traceback.format_exc().splitlines()[-3].strip()}"

if __name__ == "__main__":
    slugs = [b[0] for b in all_bodies() if b[0] != "sol" and os.path.exists(f"series/{b[0]}.json")]
    print(f"fitting {len(slugs)} bodies", flush=True)
    out = {}
    with Pool(8) as pool:
        for slug, res, err in pool.imap_unordered(fit_one, slugs):
            if err: print(f"  {slug:22s} SKIP {err}", flush=True); continue
            out[slug] = res
            print(f"  {slug:22s} {res['kind']:10s} rms {res['rms_m']/1e3:11.1f} km "
                  f"rel {res['rel_rms']:.2e}  (osculating was {res['baseline_rms_m']/1e3:.1f} km)", flush=True)
    json.dump(out, open("fits.json","w"), indent=1)
    print(f"\nfitted {len(out)}")
