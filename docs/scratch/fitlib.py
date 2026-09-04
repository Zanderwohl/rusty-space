"""Least-squares mean elements, matching em-sim's propagation model exactly.

Parameters are [n, a, e, i, raan0, argp0, M0, argp_rate, raan_rate]:
  n           mean-motion, deg/day (ANOMALISTIC when the apsides move)
  a           semi-major axis, m
  raan/argp   deg at epoch, with rates in deg/day

Precession enters as a RATE rather than a period: zero means none, where a period would
have to be infinite. Converted back to a period only when emitting Rust.
"""
import math

DAY = 86400.0
D2R = math.pi / 180.0
J2000 = 2451545.0

def kepler_E(m, e):
    m = math.fmod(m, 2 * math.pi)
    E = m if e < 0.8 else math.pi
    for _ in range(80):
        f = E - e * math.sin(E) - m
        d = f / (1 - e * math.cos(E))
        E -= d
        if abs(d) < 1e-14: break
    return E

def kepler_H(m, e):
    H = math.asinh(m / e) if abs(m) < 4 * e else \
        math.copysign(math.log(2 * abs(m) / e + 1.8), m)
    for _ in range(200):
        f = e * math.sinh(H) - H - m
        d = f / (e * math.cosh(H) - 1)
        H -= d
        if abs(d) < 1e-13: break
    return H

def model(p, t):
    """t in seconds from J2000; returns position in metres."""
    ndeg, a, e, inc, raan0, argp0, m0, argp_rate, raan_rate = p
    d = t / DAY
    m = (m0 + ndeg * d) * D2R
    if e < 1.0:
        E = kepler_E(m, e)
        nu = 2 * math.atan2(math.sqrt(1 + e) * math.sin(E / 2),
                            math.sqrt(1 - e) * math.cos(E / 2))
    else:
        H = kepler_H(m, e)
        nu = 2 * math.atan2(math.sqrt(e + 1) * math.tanh(H / 2), math.sqrt(e - 1))
    denom = 1 + e * math.cos(nu)
    # For e > 1, cos(nu) = -1/e is the asymptote where r diverges. A fit iterate can land
    # exactly there; clamp so the residual stays finite and the optimiser can back out.
    if abs(denom) < 1e-12:
        denom = math.copysign(1e-12, denom or 1.0)
    r = a * (1 - e * e) / denom
    px, py = r * math.cos(nu), r * math.sin(nu)
    w = (argp0 + argp_rate * d) * D2R
    om = (raan0 + raan_rate * d) * D2R
    i = inc * D2R
    cw, sw = math.cos(w), math.sin(w)
    ci, si = math.cos(i), math.sin(i)
    co, so = math.cos(om), math.sin(om)
    x1, y1 = cw * px - sw * py, sw * px + cw * py
    x2, y2, z2 = x1, ci * y1, si * y1
    return (co * x2 - so * y2, so * x2 + co * y2, z2)

def residuals(p, data):
    out = []
    for t, tx, ty, tz in data:
        mx, my, mz = model(p, t)
        out += [mx - tx, my - ty, mz - tz]
    return out

def rms_max(p, data):
    s = 0.0; mx = 0.0
    for t, tx, ty, tz in data:
        a, b, c = model(p, t)
        q = (a - tx) ** 2 + (b - ty) ** 2 + (c - tz) ** 2
        s += q
        if q > mx: mx = q
    return math.sqrt(s / len(data)), math.sqrt(mx)

def _solve(A, b):
    n = len(b)
    M = [row[:] + [b[i]] for i, row in enumerate(A)]
    for c in range(n):
        piv = max(range(c, n), key=lambda r: abs(M[r][c]))
        if abs(M[piv][c]) < 1e-30: return None
        M[c], M[piv] = M[piv], M[c]
        for r in range(n):
            if r == c: continue
            f = M[r][c] / M[c][c]
            for k in range(c, n + 1): M[r][k] -= f * M[c][k]
    return [M[i][n] / M[i][i] for i in range(n)]

def gauss_newton(p0, data, free, iters=60, bounds=None):
    """`free` is the list of parameter indices to vary; the rest stay fixed.

    `bounds` is an optional {index: (lo, hi)} box, clamped after each step. Without it
    the search has a degenerate minimum: when the mean motion is wrong the phase is
    uncorrelated, and the cheapest way to reduce the residual is to shrink the orbit to a
    point. Bounding `a` near its osculating value — which is well determined, unlike the
    rate — removes that escape and forces the fit to match the geometry.
    """
    p = list(p0); lam = 1e-3
    cur = sum(v * v for v in residuals(p, data))
    for _ in range(iters):
        r0 = residuals(p, data); k = len(free)
        J = []
        for idx in free:
            h = abs(p[idx]) * 1e-7 or 1e-9
            q = list(p); q[idx] += h
            rk = residuals(q, data)
            J.append([(rk[j] - r0[j]) / h for j in range(len(r0))])
        A = [[sum(J[i][m] * J[j][m] for m in range(len(r0))) for j in range(k)] for i in range(k)]
        g = [-sum(J[i][m] * r0[m] for m in range(len(r0))) for i in range(k)]
        ok = False
        for _ in range(14):
            Ad = [[A[i][j] + (lam * A[i][i] if i == j else 0.0) for j in range(k)] for i in range(k)]
            d = _solve(Ad, g)
            if d is None: lam *= 10; continue
            q = list(p)
            for slot, idx in enumerate(free): q[idx] += d[slot]
            if bounds:
                for idx, (lo, hi) in bounds.items():
                    q[idx] = min(max(q[idx], lo), hi)
            if not (0.0 <= q[2] < 20.0 and q[0] != 0.0): lam *= 10; continue
            if q[2] < 1.0 and q[1] <= 0: lam *= 10; continue
            new = sum(v * v for v in residuals(q, data))
            if new < cur:
                p, cur, lam, ok = q, new, max(lam * 0.3, 1e-11), True
                break
            lam *= 10
        if not ok: break
    return p
