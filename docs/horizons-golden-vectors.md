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
