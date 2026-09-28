# Ship fitting and energy

What a ship's energy is worth, what it weighs, and what it costs to fly.

**Status: built.** `lc_world::{fitting, cost}`, `lc_server::fitting`, the ledger (`R`) and the
Dev actions panel (`F5`). What a ship is built from is [29-ship-form.md](29-ship-form.md): parts
whose volumes are its capacities, changed by refits that run as rounds paid for in energy. This
doc is the account those parts are kept in. [30-the-field.md](30-the-field.md) is where the
energy it loses goes, and [20-solar-power.md](20-solar-power.md) is where it comes from.

Energy is the currency of everything. A ship stores it, spends it on every burn, spends it
building parts, gets most of it back by taking parts apart, and bleeds a little of it keeping its
crew alive. [03-world-model.md](03-world-model.md)'s *Resources* section says energy is consumed by
fabrication, propulsion and transmission. This is the first two; [31-directed-energy.md](31-directed-energy.md)
charges the third.

Only a player's ship carries a `Fitting`. Craft a scene stages have none and fly on
`Kind::drive()`, so the director and every demo work without one. The fitting belongs to a
`Craft`, not to its `ShipState`: `motion::apply` is pure motion and stays that way.

## The unit

Every energy in these docs is quoted in **module-energies**, ME: the mass-energy of a **slot**, a
volume of 392 699 m³ at module density. The slot is a twentieth of a 500 m hull five long by three
across by one deep, and survives as `form::presets::SLOT_M3`, the volume every density in 29 is
quoted per.

| quantity | value | from |
|---|---|---|
| slot volume | 392 699 m³ | `SLOT_M3` |
| module density | 395.8 kg/m³ | a 40 ft ISO container at its 30 480 kg maximum gross, over its external volume |
| a slot's dry mass | 1.554 × 10⁸ kg | |
| **one module-energy** | **1.397 × 10²⁵ J** | dry mass × c² |

A part costs its own mass-energy to build. Data weighs `data_mass_fraction` of module density, so
it costs that fraction of an ME per slot of it.

## Mass

**Ship mass = the form's dry mass + stored energy / c² + heat / c².**

- **Dry mass** is each part's contents at its kind's density, plus its structure by area
  ([29-ship-form.md](29-ship-form.md#hull-structure-follows-area)).
- **Stored energy is mass.** A full storage part at the default capacity weighs six times its
  dry mass, and a ship lightens as it burns, down to dry.
- **So is heat.** The field's `Q` counts at `Q / c²` ([31-directed-energy.md](31-directed-energy.md#the-drive-is-the-radiator)).
- Building converts stored energy into part mass one for one, so **a refit does not change a
  ship's mass**, except for the 5% a dismantling radiates away.

`Craft::mass_kg_at(t)` reads it, because mass changes over time. For a craft with no fitting it is
`density × volume`.

## The drive is a rocket whose exhaust is its stored energy

**m_after = m_before · exp(−Δη / ε)**

and the energy spent is `(m_before − m_after) · c²`. That is what a burn is priced and spent at.
Below ε = 1 only `ε` of it leaves as exhaust, and the rest stays aboard as heat until it radiates,
so the ship is heavier than this by that much ([30-the-field.md](30-the-field.md#conversion)).
While the drive is lit, its exhaust is drawn from the field's heat before storage
([31-directed-energy.md](31-directed-energy.md#the-drive-is-the-radiator)).

- **Δη = ∫ α dτ** — proper acceleration integrated over the ship's own clock, for as long as
  anything is lit. Rapidity, summed without regard to direction.
- **ε is `drive_efficiency`.** At 1 this is a perfect photon rocket, the best physics allows.
  Below 1 it is worse. **Above 1 is allowed** and is unphysical on purpose: it is the balance
  lever for making players more powerful, if that turns out to be needed.
- **Thrust is the aft engines' aperture over `c`** ([29-ship-form.md](29-ship-form.md#what-a-craft-reads)).
  An engine firing fore pushes the other way.

### Why not kinetic energy

Charging each burn the kinetic energy it added breaks either way, because a drive that throws
nothing out has no frame to measure kinetic energy in:

- **Measured from the burn's own start**, cost goes as Δη², so splitting a burn in `n` pieces
  costs `1/n` as much. Reaching 0.5c in one burn leaves 87% of the wet mass; in a hundred pulses,
  99.8%. In the limit it is free.
- **Measured in the world frame**, turning at constant speed changes no kinetic energy and is
  free, which is the same exploit sideways.

The rocket law is additive in Δη, so pulsing gains nothing and turning costs what accelerating
does. It is also frame-invariant and closed-form. What it gives up is the Δv² law at low speed:
cost goes as `m · Δv · c / ε`, which means **flying inside a system costs real energy**.

### Constant proper acceleration survives

Every plan in `lc-world` — `flight`, `injection`, `transfer`, `pursuit`, `escort`, `consort` —
assumes constant proper acceleration. Constant thrust on a lightening ship would break every one
of those closed forms. So:

- **The engines throttle down as mass falls**, and the crew feels the same g throughout.
- The ceiling a new order is clamped to is aft thrust over mass **at the moment the order is
  accepted**, which is the heaviest the ship will be for that plan.
- Mass during a burn is `m_start · exp(−α · τ_lit(t) / ε)`, where `τ_lit(t)` is proper time spent
  lit so far. Closed form, so reading it needs no stepping.

A ship that has drained its storage is light, so its next plan is clamped against a higher
ceiling: the starting ship pulls 5 g full and 13.8 g empty.

### What a plan costs, and when it is paid

**The whole plan is committed when the order is accepted, or the order is refused.** A ship
therefore cannot run dry halfway through a burn, and the server needs no bookkeeping per tick.

- Each plan exposes the proper time it spends lit. `Cruise` tracks boost, coast and brake proper
  time, and `Transfer`, `Rendezvous` and `Consort` are built on it. `Escort` is the exception: its
  push follows the quarry's acceleration, for as long as the quarry burns, which no plan bounds.
  Its approach is committed as usual and its station-keeping is charged as it goes, at the
  magnitude of the quarry's acceleration — an overstatement, never an understatement. The server
  breaks an escort off when its storage runs out.
- `Burn` changes velocity instantly: Δη is the rapidity of the new velocity relative to the old.
- **Refunds.** `CutDrive`, `BreakOff`, and a standing intercept being re-solved all refund the
  part of the commitment not yet flown, and a re-solved plan commits afresh. A re-solve the ship
  cannot pay for breaks off. What the field's heat pays instead of storage leaves the commitment
  the same way.
- Burn cost is priced at the mass when the account was last settled. The account settles at least
  once a game day ([20-solar-power.md](20-solar-power.md)), so what the drain takes off the mass
  meanwhile is priced in a day late, and what that over-commits comes back when the plan ends.

### The budget sets the speed

With `F` joules not already committed, the most rapidity a ship can buy is

**Δη_max = −ε · ln(1 − F / (m c²))**,

which is `ε · ln(wet / dry)` for a full ship. A crossing spends it on the match, then splits the
rest evenly between boost and brake. Rather than refusing an unaffordable crossing, **the server
lowers its speed cap**, bisecting it a fixed forty times against the plan's own cost, and returns
the lowered `max_beta` in `Accepted`, as it returns a lowered acceleration.

That puts a ceiling on the game. Storage holds 5 ME per slot of its own volume, so a ship of storage
alone, before its structure, has `wet/dry = 6`, and no ship crosses faster than
**tanh(ε · ln 6 / 2)**. Storage density and ε are the two numbers that balance travel.

## Stored energy

Stored energy and the field's heat are one account, settled together
([30-the-field.md](30-the-field.md#the-heat-account)):

- the living drain, a refit's transfers and a burn's spending are **draws on storage**, and
  conversion of what arrives refills it
- a burn spends what was committed rather than what is free
- storage never holds more than its capacity, and what cannot be stored is heat

Free energy is stored minus the commitment still outstanding. The drain draws only on free
energy; a ship whose storage is down to its commitment pays only as much of the drain as
conversion brings in.

## Flying and refitting exclude each other

- `Order::Refit` is refused while `ShipState::is_under_way()`.
- Every order that lights the drive is refused while a round runs.

A ship holding station, falling or drifting may refit. A ship hanging about beside another
(`Consort`) is under way, and has to break off first.

## Balance

One struct, `lc_world::fitting::Balance`, with a `DEFAULT`. The server holds one
(`Server::set_balance`) and **states it with every account** in `Outbound::Fitted`, for the reason
`rate` is stated with the clock: a client assuming a constant would confidently preview refits
against numbers the server does not use.

The settings here are the account's. Each kind's density and the form's settings are
[29-ship-form.md](29-ship-form.md#balance), the field's are [30-the-field.md](30-the-field.md#balance),
and directed energy's are [31-directed-energy.md](31-directed-energy.md#balance).

| setting | default | meaning |
|---|---|---|
| `drive_efficiency` | 1.0 | ε. Unbounded above |
| `recovery` | 0.95 | fraction of build energy a dismantling returns |
| `module_density_kg_m3` | 395.8 | what an ME is the mass-energy of, per slot |
| `data_mass_fraction` | 0.5 | data's density over module density, and so its build energy |
| `data_work_factor` | 3 | how many times longer data takes to build or take apart than its energy says |
| `conversion_efficiency` | 0.7 | of what is converted, the fraction stored |
| `solar_gain` | *anchored* | on the star's output; see [20-solar-power.md](20-solar-power.md) |

A craft with no data part still has `ONBOARD_DATA_BYTES`, 1 MiB, for its logs
([24-standing-instruments.md](24-standing-instruments.md)).

### What the defaults give

The starting form ([29-ship-form.md](29-ship-form.md#the-starting-form)), storage full.

| | |
|---|---|
| dry / wet mass | 2.65 × 10⁹ / 7.31 × 10⁹ kg |
| stored | 30 ME |
| acceleration, full / empty | 5 g / 13.8 g |
| Δη for a full tank | 1.02 |
| fastest crossing | 0.47c |
| 1 AU in-system at 5 g | 1.3 days, 0.84 ME |
| fastest any ship can cross | 0.71c |

| storage per slot | ε | starting ship's crossing | fastest possible |
|---|---|---|---|
| 2 ME | 1 | 0.26c | 0.50c |
| 5 ME | 1 | 0.46c | 0.71c |
| 10 ME | 1 | 0.63c | 0.83c |
| 10 ME | 2 | 0.90c | 0.98c |

## Protocol

- `Order::Refit { target: Form }` and `Order::CancelRefit`: [29-ship-form.md](29-ship-form.md#protocol-and-persistence).
- `Outbound::Fitted { ship_id, fitting, hull, field }`: the balance, the settled form, stored
  energy at `since_s`, the motive's rapidity then, the outstanding commitment, and the round under
  way as its recipe; the `Hull` measured from the form; and the field
  ([30-the-field.md](30-the-field.md#where-it-goes)). Sent after `Welcome`, after every accepted
  order and every `Flying`, as each refit step finishes, and after a grant. The client takes it
  whole.
- `Inbound::Grant { joules }`: **development only**. Honored on a server that is directing, as
  `Inbound::Stage` is, and on a shard only from an **admin**: an account whose ticket carries
  `perm` of at least 1. See [16-identity.md](16-identity.md).
- `Refusal::NoEnergy`, `Refusal::Refitting`, `Refusal::UnderWay`, and for a refit `Refusal::Short`
  and `Refusal::Form`.
- `Order::SetCourse` and `Order::Cross` carry `max_beta`: asked for by the client, and returned in
  `Accepted` lowered to what the ship could pay for.

A craft's account is saved with it, form, round and field included.

## Client

- **HUD**: stored / capacity, the commitment while one is outstanding, and net solar income beside
  it. The field's bar sits next to it ([30-the-field.md](30-the-field.md#the-field-bar)).
- **The ledger** (`Panel::Refit`, key `R`, `--panel refit`): what the ship holds — stored against
  capacity, mass, rated acceleration, length, flip time, building power, data, solar and net — and
  the round under way, with Cancel. Refits are made in the editor
  ([29-ship-form.md](29-ship-form.md#apply-and-the-ledger)).
- **Dev actions panel** (`Panel::DevActions`, key `F5`, `--panel dev`): *+1 ME*, *+10 ME*, *fill
  storage*. Development only, and a shard takes them only from an admin.
- **Flight panel**: the acceleration buttons offer what the ship is rated for now.

Every button emits an `Action`, and the ledger is a pure function of `Session` and `Ui`, so it is
tested without a window ([13-client-shell.md](13-client-shell.md)).

## Where it goes

| crate | what |
|---|---|
| `lc-world` | `fitting.rs` (balance, mass, rating, the account), `fitting/heat.rs` (the field's share of it), `cost.rs` (Δη per plan, the rocket law, budget) |
| `lc-proto` | `fitting.rs`: the account on the wire |
| `lc-server` | `fitting.rs`: acceptance, commitments and refunds, grants, refits |
| `lc-client` | `refit_panel.rs` (the ledger and the dev panel), `hud.rs` |

## Open

- **Balance is a guess.** Every default above is a first number to argue with once it can be flown.
- **Zero energy** has no consequence beyond an unpaid drain. Living space is where one would go.
- **Holding a station is free**: `motion::thrust_g` treats the milligravities as zero, and so does
  the cost.
