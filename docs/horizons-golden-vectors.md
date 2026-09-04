# Regenerating the golden ephemeris vectors

`tests/ephemeris.rs` checks the bundled solar system in
`src/body/universe/solar_system.rs` against JPL Horizons (DE441). This is the only test
that validates the orbital element *data* rather than internal consistency.

## Query parameters

| Parameter | Value | Why |
|---|---|---|
| `EPHEM_TYPE` | `VECTORS` | cartesian state, not observer tables |
| `REF_PLANE` | `ECLIPTIC` | matches the sim frame (ecliptic of J2000, Z-up) |
| `REF_SYSTEM` | `ICRF` | |
| `VEC_TABLE` | `2` | position + velocity |
| `VEC_CORR` | `NONE` | **geometric** states — no light-time or aberration |
| `OUT_UNITS` | `KM-S` | km and km/s; the sim uses metres, so multiply by 1000 |
| `CENTER` | `500@10` | Sun body centre, for heliocentric planets |
| `CENTER` | `500@399` | Earth body centre, for Luna |
| `TLIST_TYPE` | `JD` | |
| `TLIST` | `2451545.0,2460676.5` | J2000, and 2025-01-01 to expose secular drift |

Times are TDB. The sim treats `Instant` as TT seconds; TT−TDB stays under 2 ms, which is
far below the tolerances here.

## Fetch script

```bash
fetch() { # $1=label  $2=COMMAND  $3=CENTER
  curl -sS --max-time 60 -G 'https://ssd.jpl.nasa.gov/api/horizons.api' \
    --data-urlencode "format=text"          --data-urlencode "COMMAND='$2'" \
    --data-urlencode "OBJ_DATA='NO'"        --data-urlencode "MAKE_EPHEM='YES'" \
    --data-urlencode "EPHEM_TYPE='VECTORS'" --data-urlencode "CENTER='$3'" \
    --data-urlencode "REF_PLANE='ECLIPTIC'" --data-urlencode "REF_SYSTEM='ICRF'" \
    --data-urlencode "VEC_TABLE='2'"        --data-urlencode "VEC_CORR='NONE'" \
    --data-urlencode "OUT_UNITS='KM-S'"     --data-urlencode "CSV_FORMAT='YES'" \
    --data-urlencode "TLIST='2451545.0,2460676.5'" \
    --data-urlencode "TLIST_TYPE='JD'" \
  | awk -v L="$1" '/\$\$SOE/{f=1;next} /\$\$EOE/{f=0} f{print L","$0}'
}

fetch mercury '199' '500@10'
fetch venus   '299' '500@10'
fetch earth   '399' '500@10'
fetch mars    '499' '500@10'
fetch jupiter '599' '500@10'
fetch uranus  '799' '500@10'
fetch neptune '899' '500@10'
fetch luna    '301' '500@399'
```

Output columns are `JDTDB, Calendar Date, X, Y, Z, VX, VY, VZ`.

## Body codes

Major bodies use NAIF ids: Mercury `199`, Venus `299`, Earth `399`, Mars `499`,
Jupiter `599`, Saturn `699`, Uranus `799`, Neptune `899`, Luna `301`, Sun `10`.
Small bodies need a trailing semicolon: Ceres `1;`, Vesta `4;`, Eris `136199;`.

Note the bundled solar system has no Saturn.

## Refitting mean elements

Osculating elements at a single epoch are the wrong input for a fixed-element model,
especially for the Moon, whose osculating `e` swings between roughly 0.026 and 0.077 and
whose `i` swings around 5.0–5.3°. Fit mean elements against a dense position series
instead.

Fetch the series with `START_TIME`/`STOP_TIME`/`STEP_SIZE` rather than `TLIST`:

```bash
--data-urlencode "START_TIME='JD2451545.0'" \
--data-urlencode "STOP_TIME='JD2469807.5'" \
--data-urlencode "STEP_SIZE='5d'"
```

Then least-squares fit `(a, e, i, Ω₀, ω₀, M₀, apsidal_period, nodal_period)` to minimise
position residual. Two things matter:

**Fit mean motion, not `a`.** The objective is wildly multimodal in the mean motion — if
the period is off by even 0.1%, phase error exceeds a full revolution within decades and
the residuals go uncorrelated, so a gradient method collapses to a degenerate circle
(`e → 0`). Parameterise by mean motion `n` directly, and use **span continuation**: fit
over ~1.5 years first, where phase is unambiguous, then widen to 3, 6, 12, 25, 50.
Convert back with `a = (μ/n²)^⅓` at the end.

**Mean anomaly advances at the anomalistic rate.** When periapsis precesses, `M` is
measured from a moving reference, so `n` is the anomalistic mean motion, not the sidereal
one. The Luna fit recovers 27.554525 d against the true anomalistic month of 27.554550 d
(2 seconds), where the sidereal month is 27.321661 d. Because the model computes
`n = sqrt(μ/a³)`, `a` must absorb this difference and ends up 0.66% above the true value.

**`argument_of_periapsis` precesses faster than the longitude of perihelion.** ω is
measured from the node, which itself regresses, so
`dω/dt = dϖ/dt − dΩ/dt = 0.1114 + 0.0530 = 0.1643 °/d` — a period of 2190 d, not the
familiar 3232 d (8.85 yr) of ϖ. Fitting recovers 2190.5 d.

Luna result over 2000–2050: RMS 7 989 km, max 15 556 km (~1.2°). That is the floor for
this model class — evection alone is 1.27° and is not representable by a precessing
ellipse.
