# Overview

## What the game is

A real-time strategy sandbox set across a volume of real stars. Each player controls one
ship. The ship mines, refines, builds, and launches further ships and structures. There is
no tech tree gate on scale: a player can end up running a von Neumann fleet across several
systems.

The rule that shapes everything else: **information propagates at c**. Every player action
is an event with a spacetime coordinate. A player only knows about an event once its light
cone has reached an instrument they own. What a player sees of a distant star is what that
star emitted years ago.

## The rate

The server runs one in-game Julian year per real hour.

```
31 557 600 s / 3600 s = 8766x
```

Consequences, which are the actual design of the game:

| in-game | real time |
|---|---|
| 1 light-year of travel | 1 hour |
| Sun to Earth (499 s) | 57 ms |
| Sun to Neptune (4.2 h) | 1.7 s |
| Sun to 1000 AU | 5.7 min |
| Proxima Centauri, one way (4.246 ly) | 4.25 h |
| Proxima round trip | 8.5 h |

So: **in-system play is real-time and latency-free; interstellar play is asynchronous and
measured in hours.** A player issuing an order to a probe at Proxima gets confirmation the
next day. This is not a limitation to be engineered around. It is the game.

The rate is a server configuration value. Tuning it changes the genre — at 1 year per
minute the nearest star is 4 minutes away and the game becomes an RTS; at 1 year per day
it becomes play-by-mail. Everything in these documents assumes 8766x unless stated.

## Scope

In:

- Special relativity. Light delay, Doppler, aberration, ship proper time.
- Newtonian and Keplerian orbital mechanics, reusing `em-foundations` and `em-sim`.
- Passive observation: photometry, transit detection, spectroscopy.
- Active signaling: omnidirectional radio, tight-beam, and the detectability difference.
- Resource extraction, construction, self-replicating probes.
- One authoritative server. Browser (WASM) and native desktop clients.

Out:

- General relativity. No curvature, no frame dragging, no gravitational lensing or
  redshift. Gravity is Newtonian for trajectories and absent from the light model.
- FTL of any kind, including sensors, communications and drives.
- Avatar-scale play. The smallest controllable unit is a ship or a structure.
- Combat as a primary loop. Interception exists because it falls out of the physics;
  it is not the subject of the game.

## Non-goals that could be mistaken for goals

- **Player-visible time dilation.** Ships at relativistic speed accumulate less proper
  time, and that governs onboard process rates. The player's clock is always the server
  frame. There is no per-player frame to synchronise, and no player ever sees a Lorentz
  transform of the world. See [01-spacetime.md](01-spacetime.md).
- **Full-sphere radiance simulation.** The emission shell around each star is a data model
  for what light leaves the system in each direction. It is evaluated at the directions
  actually observed, not integrated over the sphere. See
  [04-stellar-photometry.md](04-stellar-photometry.md).
- **Simulating everything continuously.** Orbits are analytic. A body's state at any
  coordinate time is a closed-form evaluation, so nothing needs stepping and nothing needs
  storing per tick. Only discrete state changes become events.

## Primary technical risks

| risk | where it is addressed |
|---|---|
| Light-cone queries over a large event table are not indexable in general | [02-event-store.md](02-event-store.md) — query sources, not events |
| f64 meters lose precision at interstellar range | [01-spacetime.md](01-spacetime.md) — integer light-microsecond grid |
| Per-element occultation does not scale to swarms of 1e6+ | [04-stellar-photometry.md](04-stellar-photometry.md) — populations are distributions, not rosters |
| Browser targets restrict shaders and transport | [07-rendering.md](07-rendering.md), [08-networking.md](08-networking.md) |
| Client-side prediction must agree with the server bit-for-bit where rules depend on it | [08-networking.md](08-networking.md) — shared `libm` |
