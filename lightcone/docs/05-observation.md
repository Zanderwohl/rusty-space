# Observation and signalling

Everything a player learns about a distant system arrives through one of these channels.
Each is subject to light delay, and each has a detection threshold that turns into a
gameplay cost.

## Photometry

A telescope points at a star and records flux over time. The flux is
`L(n, t_r) / d^2` from [04-stellar-photometry.md](04-stellar-photometry.md).

### Sensitivity

Photon-limited. With aperture area `A`, throughput `eta`, integration time `T`, and source
photon flux `F` (photons per square metre per second):

```
N     = F * A * eta * T
d_min = 1 / sqrt(N)               smallest fractional depth resolvable at SNR 1
```

Reference numbers, V band, where `V = 0` is about `1e10 photons/m^2/s`:

| source | apparent V | photon flux |
|---|---|---|
| Sun-like star at 10 pc | 4.83 | 1.2e8 /m^2/s |
| Sun-like star at 100 pc | 9.83 | 1.2e6 /m^2/s |
| Sun-like star at 1 kpc | 14.83 | 1.2e4 /m^2/s |

Transit depths are `(R_body / R_star)^2`:

| occluder around a Sun-like star | depth |
|---|---|
| Jupiter | 1.06e-2 |
| Neptune | 1.3e-3 |
| Earth | 8.4e-5 |
| swarm covering 1% of the sky as seen from the star | 1e-2, steady |
| 1.5e6 collectors of 1e6 km^2 at 1 AU | 5.3e-6 mean, 1.9e-6 rms flicker |

Required exposure for an Earth-analogue at 10 pc with `eta = 0.5`:

```
N = 1 / (8.4e-5)^2 = 1.4e8 photons
T = N / (F * A * eta) = 1.4e8 / (1.2e8 * A * 0.5) = 2.4 / A   seconds of in-game time
```

So aperture is not the binding constraint nearby. **Telescope time is.** An Earth-analogue
transits for 13 hours once per year with a duty cycle of 0.0015, and confirming a period
takes three transits — three in-game years, three real hours of continuous pointing at one
star. Aperture becomes binding with distance, because `F` falls as `1/d^2`: the same
detection at 100 pc needs 100x the aperture-time product, and at 1 kpc, 10 000x.

That is the intended shape of the mechanic. A small telescope surveys nearby stars. Reaching
further means building a bigger one, or many, and committing them for in-game years.

### Survey regimes

A telescope, or a group of them acting together, commits to a regime for a period of time.

| regime | targets | depth per target |
|---|---|---|
| stare | one star | the whole integration |
| field | a region of sky | `T / N_field` |
| all-sky survey | everything above the horizon of the instrument | `T / N_total` |

Depth goes as `sqrt(T)`, so the tradeoff is exact: covering `N` times as many targets costs a
factor of `sqrt(N)` in the smallest depth detectable. A survey that finds where to look and a
stare that finds what is there are different instruments used at different times, not
different hardware.

**Calling a commitment off early does not cancel it. It leaves the result uncertain.** A
half-finished integration produces a measurement with error bars, not nothing — a 2-sigma bump
that might be a planet, might be a swarm, might be noise. That is the correct output and it
should be shown as such.

This is the single most important UI decision in the observation system: **the client reports
a measurement and its uncertainty, and the player decides what to believe.** It does not report
"planet detected". A player acting on a 2-sigma bump is making a real decision under real
uncertainty, and being wrong sometimes is the mechanic working.

Server-side confirmation still applies where a rule depends on a discovery, and it is
consistent with this: noise is seeded on `(telescope, star, time bucket)`, so the server can
recompute exactly what the instrument saw and check the claim against it. The player's
uncertainty is real; the server's is not.

### Period finding

Two tools, for two kinds of signal. Use the right one.

| signal | shape | tool |
|---|---|---|
| stellar rotation, pulsation | near-sinusoidal, high duty cycle | FFT / Lomb-Scargle |
| swarm or belt flicker | stationary noise with a spectral knee | power spectrum, fitted |
| planetary transit | box-shaped, duty cycle 1e-3 to 1e-2 | box least squares (BLS) |

An FFT is the wrong instrument for a narrow transit: the power of a box of duty cycle `q`
spreads across `~1/q` harmonics, so a signal that BLS finds at SNR 10 is scattered below the
noise floor in a periodogram. Implement both; expose both; let a player who runs an FFT on a
transit see a forest of harmonics and learn why.

Unevenly sampled series — which is what a telescope with gaps produces — need
Lomb-Scargle rather than a plain FFT. Assume gaps; the sampling is driven by whatever else
the player was doing.

A population is the case where the FFT is not looking for a period at all. A swarm produces
no line, because its elements are uniformly distributed in phase; it produces **stationary
noise with a mean offset**, and the shape of that noise carries the information. The power
spectrum is flat below a knee at `f = v_perp / (2 * pi * R_star)` and falls above it, so
fitting the knee gives the orbital velocity and therefore the semi-major axis, without any
periodicity existing to find.

This is the transition worth building the UI around. A star with four planets shows four
lines in a periodogram. A star with a swarm shows no lines and a raised, structured noise
floor. A star mid-construction shows both, and the lines disappearing one by one into the
floor as the swarm fills in is the most legible signal in the game that someone is building
something.

### What is inferable

Coherent occluders, from a periodogram or a BLS search:

| measurement | yields |
|---|---|
| period `P` | semi-major axis, via the star's mass and Kepler's third law |
| depth `d` | occluder radius |
| duration and ingress shape | impact parameter, therefore inclination |
| depth versus band | occluder temperature; a solid body and a hot structure differ |
| IR excess | waste heat, therefore engineering rather than a planet |

Populations, from the first two moments of the light curve and the spectral knee. Using
`d` for the mean deficit and `f` for the rms flicker, as derived in
[04-stellar-photometry.md](04-stellar-photometry.md):

| measurement | yields |
|---|---|
| mean deficit `d` | covering fraction, `N * sigma / (4 pi a^2)` |
| `k = f^2 / d` | deficit contributed by one element |
| `sigma = k * pi * R_star^2` | **size of an individual element** |
| `m = d / k` | elements on the disc at any instant |
| spectral knee | orbital velocity, therefore `a` |
| `N = 4 m a^2 / R_star^2` | **total element count** |
| deficit versus latitude, across several observers | the population's inclination spread and pole |

Element size and element count separate because the mean and the variance of a Poisson
process scale differently. One telescope, one long enough stare, and an observer knows how
many objects of what size orbit a star 30 light-years away — not merely that something is
there.

All at the retarded time. A measurement of a system 30 ly away describes that system 30
in-game years ago, and the UI must label it that way everywhere, without exception.

## Spectroscopy

Same light, dispersed. Gives effective temperature, composition of any transiting
atmosphere, and radial velocity via Doppler shift. Cost scales the same way but needs more
photons per resolution element, so it is roughly `R` times more expensive than photometry at
resolving power `R`. Treat as photometry with a large constant factor.

## Radio

### Isotropic broadcast

Flux at distance `d` from an isotropic transmitter of power `P` is `P / (4 pi d^2)`.
Detection uses the radiometer equation: with system temperature `T_sys`, bandwidth `B`,
integration `tau`, and effective area `A_eff`,

```
S_min = 2 * k * T_sys / (A_eff * sqrt(B * tau))
```

The consequence for the game is the fan-out bound in
[02-event-store.md](02-event-store.md): every emission has a finite detection radius, so the
set of receivers is finite and can be enumerated at write time.

Isotropic broadcast is cheap to send, reaches everyone in range, and announces the sender's
position to every one of them. It is the default for a player who has not thought about it.

### Tight beam

Beamwidth is diffraction-limited: `theta ~ lambda / D`. The spot size at the target is
`theta * d`.

| emitter | wavelength | aperture | beamwidth | spot at 4 ly |
|---|---|---|---|---|
| radio dish | 0.30 m (1 GHz) | 10 m | 3.0e-2 rad | 1.1e15 m = 7600 AU |
| radio dish | 0.03 m (10 GHz) | 30 m | 1.0e-3 rad | 3.8e13 m = 250 AU |
| optical laser | 1.0e-6 m | 1 m | 1.0e-6 rad | 3.8e10 m = 0.25 AU |

This produces the covert-communication rule without inventing one. A radio "tight beam"
across 4 light-years still floods the entire target system and a large volume around it: it
is not covert, it is only efficient. An optical link at the same range lands a spot of
0.25 AU, so it is intercepted by anything inside the target's inner system and by nothing
else. Covertness is bought with wavelength and aperture, and the numbers say how much.

### Choosing a frequency

The sky is not equally noisy everywhere. Galactic synchrotron falls as roughly `nu^-2.7`,
the CMB contributes a flat 2.725 K, and quantum noise rises as `h nu / k`. The sum has a
minimum:

| frequency | synchrotron | quantum | total sky |
|---|---|---|---|
| 400 MHz | 297 K | 0.02 K | 299 K |
| 1.42 GHz | 9.7 K | 0.07 K | 12.5 K |
| 5 GHz | 0.32 K | 0.24 K | 3.3 K |
| 10 GHz | 0.05 K | 0.48 K | 3.3 K |
| 30 GHz | 0.00 K | 1.44 K | 4.2 K |
| 100 GHz | 0.00 K | 4.80 K | 7.5 K |

The free-space microwave window is about 3 to 30 GHz, with a floor near 3.2 K.

**1420 MHz is not in it.** The hydrogen line sits at 12.5 K, four times the floor, which costs
a factor of 15 in transmitter power or in integration time for the same detection. It is the
worst good choice and the best obvious one — everyone knows where it is, so it is where a
civilisation broadcasts if it wants to be found and where it listens if it expects company. A
transmitter that wants to be heard pays the 15x. A transmitter that wants privacy moves to
8 GHz and accepts that nobody is listening there unless told to.

That tension is the whole frequency mechanic, and it is derived rather than assigned:

| choice | cost | consequence |
|---|---|---|
| 1420 MHz, or the band up to the OH lines at 1720 MHz | 15x power | found by anyone surveying; the default meeting place |
| 3-30 GHz window | none | heard only by a receiver already pointed and tuned |
| deliberately obscure | none | requires pre-arrangement, which requires a prior message |

Pre-arranging a private frequency requires sending a message to arrange it, and that message
has to go somewhere. First contact is loud by necessity.

### Aiming

A beam must be aimed where the target **will be** when the light arrives, not where it is
seen to be. The sender solves the advanced-time problem — the mirror of the retarded-time
solve in [01-spacetime.md](01-spacetime.md):

```
find t_a such that   t_a = t_send + |x_target(t_a) - x_send|
```

using the sender's *predicted* worldline for the target, which is built from observations
that are themselves old. A target that manoeuvres after the light left, but before it
arrives, is missed. For a ship 4 ly away this means the aim is based on where the target was
4 years ago, extrapolated 4 years forward — 8 years of prediction error. Stationary
installations are easy to hit; ships under thrust are not.

## Interferometry

Two instruments separated by a baseline `B` resolve `lambda / B`. At the scales available here
that number stops being the limit almost immediately.

| baseline | band | resolution | linear size resolved at 10 ly |
|---|---|---|---|
| 1 AU | 550 nm | 3.7e-18 rad | 0.35 m |
| 10 AU | 550 nm | 3.7e-19 rad | 3.5 cm |
| 1 AU | 21 cm | 1.4e-12 rad | 133 km |
| 4 ly | 21 cm | 5.6e-18 rad | 0.53 m |

Resolving 35 cm features on a planet ten light-years away is not a game mechanic, so
**diffraction is not what limits interferometry. Photons are.** An `N x N` image needs `N^2`
times the photons of a single measurement at the same SNR, so collecting area scales as the
square of the linear resolution wanted. Angular resolution is free; surface brightness is not.
That is the correct limiter, it is physical, and it makes imaging an endgame investment
measured in square kilometres of aperture rather than in baseline length.

### Within a system

Practical. Baselines of an AU or more, elements in known orbits, path lengths derivable from
the same orbital mechanics everything else uses. A battery of telescopes in a common orbit is
a natural structure and a natural thing to build incrementally: each element added improves
the `u-v` coverage and the collecting area at once.

### Between systems

Optical is impossible and radio is merely very hard, and the difference is worth stating
because it is what makes the 21 cm band matter.

Coherent combination needs the path difference known to about `lambda / 10`:

| band | baseline | absolute precision needed | relative |
|---|---|---|---|
| optical | 1 AU | 5.5e-8 m | 3.7e-19 |
| optical | 4 ly | 5.5e-8 m | 1.5e-24 |
| 21 cm | 4 ly | 2.1e-2 m | 5.6e-19 |

Knowing an interstellar baseline to 55 nanometres is not a matter of effort. Knowing it to
2 centimetres is the same class of problem terrestrial VLBI already solves, and the game has
an advantage real VLBI does not: every station's position comes from the simulation, and
every station's proper time is already related to server time by the engine.

The other half of radio VLBI is that the waveform can be recorded. At optical frequencies it
cannot — 5e14 Hz is not sampleable — but 21 cm is, and the recording is then a data volume
problem:

| bandwidth | sample rate | at 2 bytes/sample |
|---|---|---|
| 1 MHz | 2 Msamp/s | 4 MB/s, 0.3 TB per in-game day |
| 100 MHz | 200 Msamp/s | 400 MB/s, 35 TB per in-game day |

**So interstellar interferometry is: observe, record, ship the recordings to a correlator, and
get the image years later.** The data streams have to reach a common node, which takes light
travel time, and they have to be associated across the times at which each was taken, which
the event store already does — a recording is an event with a coordinate, and correlating two
of them is a query about two worldlines.

That is a very good late-game mechanic. It is also the only thing in the design where a player
deliberately builds infrastructure whose payoff arrives a decade after the observation, and it
fits the premise better than anything else on the list.

## Instrument placement and the double delay

A telescope at another star does not give a player a live view of that star's neighbourhood.
It gives a view delayed by the telescope-to-target distance, and then the *report* is delayed
by the telescope-to-player distance. Total lag is the sum, not the maximum.

A network of telescopes therefore has a geometry problem with no clean answer, which is the
point: forward observatories see sooner but report later, and the optimum depends on which
direction the player expects news to come from.

## Client and server split

| step | where |
|---|---|
| light curve generation | client, from the star's emission model and the known occluders |
| noise realisation | client, from a seed derived from `(telescope, star, time bucket)` |
| FFT / BLS / Lomb-Scargle | client, in the UI, on demand |
| what a player has *discovered* | server, because it gates rules and must not be forgeable |

Generation is deterministic and duplicated, so the client can draw a curve at any zoom level
without asking. Anything that changes rules — a detection that unlocks knowledge of a rival's
position — is confirmed server-side from the same deterministic functions.

## Open

- Noise model beyond photon statistics: detector read noise, zodiacal background, and
  scintillation if any instrument is ever atmospheric. Photon noise alone makes large
  apertures too strong.
- Whether spectroscopy gets its own instrument type or is a telescope mode.
- Interferometry within a system is settled in outline above. What is not: the `u-v` coverage
  model, how partial coverage degrades an image, and the correlator's cost. Interstellar VLBI
  needs a concrete data-shipping mechanic — whether recordings travel as cargo, as transmitted
  signal, or both.
- Neutrino and gravitational-wave channels. Both travel at c and both would bypass
  occlusion, which may be more mechanic than the game needs.
