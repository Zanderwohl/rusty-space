# Authoring system contents

How a designer shapes what orbits the stars of an authored galaxy: planets, belts, Oort clouds and
life, and a setting that guarantees ocean worlds where they could exist. The galaxy decides each
star (see [34-galaxy-authoring.md](34-galaxy-authoring.md)), and the system generator turns a star
into its system (see [26-system-generation.md](26-system-generation.md)). This document is the part
between them.

**Status: S1–S3 built.** The system fields, the per-chunk profile table (chunk format version 2),
the profile on `CatalogStar` and the ocean-first construction are in. S4, Galaxy Forge's reporting,
is not.

The design in one paragraph: the galaxy's feature graph gains a small set of **system fields**,
which are soft-edged, region-level values like the `[Fe/H]` offset it already has. The bake samples
them at each star and stores the result in the published record, as an index into a per-chunk table
of **system profiles**. When the shard or a client generates a star's system, it builds a `Tuning`
from that profile instead of using the global default. One of the fields, `ocean`, is the
probability that an eligible star generates an ocean world first and builds the rest of its system
around it. At 0 the generator runs exactly as it does today.

## The generator stays in charge

Authoring does not place planets. The generator already gets the solar system, the asteroid belt's
missing mass, the giant–metallicity correlation and lonely hot Jupiters out of one temperature
curve, one mass budget and one retention rule. Hand-placed planets would break mass conservation
and the classifier's prior ([26](26-system-generation.md), "Reading it back"), and would cost a
designer per-star work the galaxy cannot afford.

So every system field moves one of the generator's existing dials, by region:

| field | range | what it moves in `Tuning` | default |
|---|---|---|---|
| `ocean` | 0–1 | the chance an eligible star builds an ocean world first; see below | 0 |
| `solids` | ±1.5 dex | `disc.solid_earths`: richer or poorer discs, so more or fewer and heavier or lighter planets and giants | 0 |
| `belts` | ±1.5 dex | `belts.belt_survival` and `belts.kuiper_survival`: how much of a stirred zone or the outer disc survives as a belt | 0 |
| `cloud` | ±1.5 dex | `belts.oort_efficiency`: how much a system's giants throw into its Oort cloud | 0 |
| `life` | 0–1 | `world.life_fraction`: the share of habitable worlds something lives on | 0.5 |

`solids` already rides on metallicity, because the disc's solid mass scales with `10^[Fe/H]`. It is
a separate field because a designer who wants a planet-rich region should not have to make it
metal-rich as well, which would also shift its giants and its stars' colors.

The list is deliberately short. Each field is a byte of state per profile and a column in the
editor's statistics, and the generator's other dials (the snow line, the retention rule, the
mass–radius anchors) are measured quantities that a region should not be able to move.

## In the feature graph

System fields are new `Target` variants beside `Density`, `Exclusion` and `FeH`, and they combine
the same way: a shape contributes `(Target, Op)` pairs, terms apply in timeline order, and every
edge is soft. A nebula that is rich in planets and life, inside a region that favors ocean worlds,
is three contributions:

```ron
Feature(id: 9, kind: "ellipsoid", version: 1, name: "Cradle", params: (frame: 0, center_ly: (40, 12, 0), radii_ly: (30, 20, 8), edge_ly: 10,
    contributes: [(System(Ocean), Set(1.0)), (System(Solids), Add(0.3)), (System(Life), Set(0.8))]))
```

Unlike the density targets, system fields are not per population. A system belongs to a star, and
the field value is read at the star's position. Values are clamped to the ranges in the table after
every term has applied.

## From the bake to the generator

Systems are generated on demand by the shard and the clients, which do not have the graph. The field
values therefore have to travel with the star, the way its draws do.

**A per-chunk profile table.** The bake samples the five fields at each star and quantizes them:
`ocean` and `life` in sixteenths, the three dex offsets in steps of 0.1 dex. It collects the
distinct quantized tuples in each chunk into a table in the chunk header, with the all-default tuple
always at index 0, and stores the index in a new record byte. The fields are smooth and regions are
large, so a chunk holds a handful of distinct profiles. More than 255 is a bake error naming the
chunk, and the remedy is wider edges or coarser steps.

The record grows from 16 to 17 bytes, and the chunk format moves to version 2. A whole profile per
star would cost 3 bytes; a table index costs 1. If all of tier B's 1.5e11 stars were published, the
two bytes saved per star would be 300 GB.

**Into `Tuning`.** `CatalogStar` gains an optional system profile, absent for HYG stars and for any
star whose profile is index 0. `LocalSystem::for_star` builds a `Tuning` from `Tuning::default()`
and the profile and calls `generate::system_with`, which already takes a tuning. Sol keeps its JPL
preset whatever its profile says.

## The ocean-world setting

Systems are skewed in the player's favor, as [26](26-system-generation.md) already does with its
optimistic habitable zone, generous solid mass and a life fraction of a half. `ocean` is the
authored version of that skew.

### Definitions

An **ocean world** is a rocky planet that passes [26](26-system-generation.md)'s habitability test
(in the zone, a surface, air over it, water on it) with a water fraction at least Earth's,
2.3 × 10⁻⁴ of its mass, so it has open seas rather than damp ground. The generator has no such type
today; it is a threshold on two values it already computes.

A star is **eligible** when it could generate one at all:

1. It is on the main sequence. A giant's habitable zone has moved out past where its planets
   formed, and a remnant has none.
2. Its disc can build one. The solid mass of the habitable-zone annulus, from the generator's own
   disc and drift rules at the star's mass and metallicity, is at least the smallest mass that holds
   air and water at habitable-zone temperatures under the retention rule, with a margin of 1.3 so
   the planet stage's radius spread cannot push it under. `ocean::sites` computes that floor per
   orbit by bisection. This is the "enough metallicity" condition, stated in the generator's terms:
   a metal-poor star fails it because its disc is too thin, and a red dwarf fails it at a higher
   metallicity than a Sun-like star because its disc is smaller.

Both conditions are deterministic functions of the star, so the editor can count eligible stars
without generating any system.

### The draw

Each eligible star draws one uniform number from its seed, under a salt nothing else uses. If the
number is below the star's `ocean` value, the system is built **ocean-first**; otherwise it is built
exactly as it would be at 0, using the same draws as today.

The separate salt is what makes 0 mean "no effect" and keeps every other system unchanged when a
designer raises the value: a star that loses the draw builds the system it always built, and a star
that wins it builds a different one. At 1, every eligible star is ocean-first. At 0, ocean worlds
still occur at the generator's natural rate. Measured over 300 stars per row at default tuning:

| star | [Fe/H] | eligible | ocean world at 0 | at 1 |
|---|---|---|---|---|
| red dwarf | −1.0 | 2% | 1% | 3% |
| red dwarf | −0.5 | 41% | 5% | 42% |
| red dwarf | 0.0 | 96% | 23% | 96% |
| Sun-like | −1.0 | 80% | 15% | 80% |
| Sun-like | 0.0 | 100% | 53% | 100% |
| A star | 0.0 | 100% | 40% | 100% |

The rate at 1 equals the eligible share, which is what the setting promises.

### Ocean-first construction

The ocean world is placed before anything else, and the rest of the system is built around it:

1. **Orbit.** One of 32 log-spaced habitable-zone orbits, drawn from those where the floor is under the ceiling.
2. **Mass.** Log-uniform between the orbit's floor and the smaller of what the narrowest feeding
   zone can assemble and 0.95 of the core mass at which a rocky core binds a hydrogen envelope and
   becomes a sub-Neptune. That ceiling
   is why a habitable world has to be a small one, as [26](26-system-generation.md) notes.
3. **Feeding zone.** The planet's rung sweeps its own annulus, and its mass is taken from it, so mass
   is still conserved across the disc.
4. **Water and field.** The rotation is clamped to 0.9 of the dynamo's spin limit and the field is
   switched on, so the planet keeps its water, and the delivered water is drawn log-uniform from the
   seas-or-more part of the generator's range. A
   giantless system would otherwise deliver no water at all; here the ocean world gets its seas
   anyway, which is the cheat.
5. **Life.** Drawn as for any habitable world, at the star's `life` value.
6. **The rest of the ladder.** Rungs are laid inward and outward from the ocean world's rung with
   the usual spacing draws, instead of outward from the disc's inner edge. Then everything the
   generator does after the ladder runs as usual, with three exceptions that protect the ocean world:
   - a migrating giant stops at the first stable orbit outside it instead of sweeping through;
   - a gas giant's resonance band does not stir its rung;
   - an unstable pair that includes it merges the other planet into its neighbor instead.

Belts and the Oort cloud then come out of what is left, as they always do, so an ocean-first system
can have fewer belts than its neighbors if its giants were pushed outward.

## In Galaxy Forge

Systems are cheap to generate, so while the live bake is under its star budget the editor generates
the system of every baked star and reports what the fields did:

- **Statistics:** per population and in total, planets per star, giants per star, stars with a belt,
  stars with an Oort cloud, habitable worlds, ocean worlds, eligible stars, and ocean-first systems.
- **Charts:** ocean worlds per star against `[Fe/H]` and against stellar mass, with eligible stars
  as the denominator, which shows where the metallicity condition cuts in.
- **Inspector:** the picked star's system as a list (orbit, mass, type, water, life), with the
  ocean-first marker.
- **Views:** a color-by mode for any system field, so a designer sees the regions they drew.

Above the budget, the editor counts eligible stars from the fields and the star draws alone, and
estimates ocean worlds as eligible stars times `ocean`, plus the natural rate below it.

## Changing the generator

A published star stores its profile, and its system is generated from that profile and the
generator's code, so a change to the generator changes the system of every published star. During
development that is handled the way any other change to the world is: the dev shard is wiped and
the galaxy republished. Nothing in this design is versioned.

## Order of work

| step | built |
|---|---|
| S1 | `System(...)` targets in `lc-galaxy`, clamped and sampled at each star; the profile table and the record byte; chunk format version 2 |
| S2 | the profile on `CatalogStar`, and `LocalSystem::for_star` building a `Tuning` from it; `solids`, `belts`, `cloud` and `life` take effect |
| S3 | eligibility and the ocean-first construction in `lc_world::sky::generate`, behind the separate salt; a test that `ocean = 0` reproduces today's systems for a thousand stars bit for bit |
| S4 | Galaxy Forge statistics, charts, inspector and color-by for the system fields |

S1–S3 can go to Phase II's playtest, and S4 is what makes them tunable.

## Open

| question | notes |
|---|---|
| The seas threshold | Earth's water fraction is the proposed line between an ocean world and a damp one. A higher line makes ocean worlds wetter and rarer at `ocean = 0`; the setting is unaffected. |
| Binary companions | The generator gives a binary the primary's system unchanged ([26](26-system-generation.md), Open). An ocean-first world in a binary inherits that gap. |
| More than one ocean world | `ocean` guarantees at most one per star. A second field for the chance of a second one is possible, but each extra planet placed first constrains the ladder more. |
