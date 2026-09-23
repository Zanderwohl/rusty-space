# Ship fitting and energy

What a ship is made of, what it costs to change that, and what it costs to fly.

**Status: built.** `lc_world::{fitting, cost, refit}`, `lc_server::fitting`, and the Refit
(`R`) and Dev actions (`F5`) panels. Where the build departed from the plan, this says what was
built.

Energy is the currency of everything. A ship stores it, spends it on every burn, spends it
building modules, gets most of it back by taking modules apart, and bleeds a little of it
keeping its crew alive. [03-world-model.md](03-world-model.md)'s *Resources* section says energy
is consumed by fabrication, propulsion and transmission. This is the first two. Collecting
energy comes later; until then a development button hands it out.

## Modules

A hull is divided into **slots**, and each module fills one. There are four kinds to begin with:

| module | what it does |
|---|---|
| **storage** | holds energy, up to `storage_per_module` module-energies each |
| **drone** | builds and dismantles modules; the refit rate is proportional to how many there are |
| **living** | drains `living_drain_w`, continuously, per module. Nothing else yet |
| **engine** | a fixed thrust each; acceleration is total thrust over mass |
| **data** | holds a craft's knowledge — files and raw logs — up to `data_per_module` each, with a fullness exactly as storage has; see [24-standing-instruments.md](24-standing-instruments.md). Half the mass of any other module, so half the energy to build, and three times as long |

A `Loadout` is five counts and a slot total. The starting ship has one data module, in
the slot a second living module used to fill: living space does nothing yet but drain. The
engine rating is derived from the starting ship's full mass, so it still pulls 5 g full. It belongs to a `Craft`, not to its `ShipState`:
`motion::apply` is pure motion and stays that way. Only a player's ship gets a loadout. Craft a
scene stages have none and keep flying on `Kind::drive()`, so the director and every demo work
as they do now.

## Sizing

The 500 m ovoid holds **20 slots**. A slot is a twentieth of that hull:

| quantity | value | from |
|---|---|---|
| hull volume, 500 m | 7.854 × 10⁶ m³ | `Craft::volume_m3` |
| slot volume | 392 699 m³ | ÷ 20 |
| module density | 395.8 kg/m³ | a 40 ft ISO container at its 30 480 kg maximum gross, over its external volume |
| module dry mass | 1.554 × 10⁸ kg | |
| **one module-energy** | **1.397 × 10²⁵ J** | dry mass × c² |

A module costs its own mass-energy to build. That is the unit every other energy in this doc is
quoted in, abbreviated **ME**. A data module weighs `data_mass_fraction` of the others, so it
costs that fraction of an ME.

**The slot count is the truth and the length follows from it**, not the other way round:
`length_m` is derived as the ovoid whose volume is `slots × slot_volume`. Dividing a volume by a
slot volume and flooring puts the reference hull at 19.999 999 slots. Everything that already
reads `length_m` — slew rate, plume, zoom limits, the size in a `Presence` — then follows a
hull resize with no further work.

## Mass

**Ship mass = Σ module dry mass + slots × hull structure mass + stored energy / c².**

- Hull structure is `hull_density × slot_volume` per slot. The old `Kind::Ship` density of
  250 kg/m³ averaged decks and tankage, which are modules now, so the frame on its own defaults to
  **50 kg/m³**: 1.96 × 10⁷ kg a slot, or 0.126 ME.
- **Stored energy is mass.** A full storage module at the default capacity weighs six times its
  dry mass, and a ship lightens as it burns, down to dry.
- Building converts stored energy into module mass one for one, so **a refit does not change
  a ship's mass**, except for the 5% a dismantling radiates away.

`Craft::mass_kg()` becomes `mass_kg_at(t)`, because mass now changes over time. For a craft with
no loadout it is `density × volume`, as it is now.

## The drive is a rocket whose exhaust is its stored energy

**m_after = m_before · exp(−Δη / ε)**

and the energy spent is `(m_before − m_after) · c²`.

- **Δη = ∫ α dτ** — proper acceleration integrated over the ship's own clock, for as long as
  anything is lit. Rapidity, summed without regard to direction.
- **ε is `drive_efficiency`.** At 1 this is a perfect photon rocket, the best physics allows.
  Below 1 it is worse. **Above 1 is allowed** and is unphysical on purpose: it is the balance
  lever for making players more powerful, if that turns out to be needed.

### Why not kinetic energy

The first draft charged each burn the kinetic energy it added, at 99% conversion. A drive that
throws nothing out has no frame to measure kinetic energy in, and both choices break:

- **Measured from the burn's own start**, cost goes as Δη², so splitting a burn in `n` pieces
  costs `1/n` as much. Reaching 0.5c in one burn leaves 87% of the wet mass; in a hundred pulses,
  99.8%. In the limit it is free.
- **Measured in the world frame**, turning at constant speed changes no kinetic energy and is
  free, which is the same exploit sideways.

The rocket law is additive in Δη, so pulsing gains nothing and turning costs what accelerating
does. It is also frame-invariant and closed-form. What it gives up is the Δv² law at low speed:
cost goes as `m · Δv · c / ε`, which means **flying inside a system costs real energy**, where the
kinetic model made it free to about eight places.

### Constant proper acceleration survives

Every plan in `lc-world` — `flight`, `injection`, `transfer`, `pursuit`, `escort`, `consort` —
assumes constant proper acceleration. Constant thrust on a lightening ship would break every one
of those closed forms. So:

- **The engines throttle down as mass falls**, and the crew feels the same g throughout.
- The ceiling a new order is clamped to is `engines × engine_thrust_n / mass` **at the moment the
  order is accepted**, which is the heaviest the ship will be for that plan.
- Mass during a burn is `m_start · exp(−α · τ_lit(t) / ε)`, where `τ_lit(t)` is proper time spent
  lit so far. Closed form, so reading it needs no stepping.

A ship that has drained its storage is light, so its next plan is clamped against a higher
ceiling: the starting ship pulls 5 g full and 13.6 g empty.

### What a plan costs, and when it is paid

**The whole plan is committed when the order is accepted, or the order is refused.** A ship
therefore cannot run dry halfway through a burn, and the server needs no bookkeeping per tick.

- Each plan exposes the proper time it spends lit. `Cruise` already tracks boost, coast and brake
  proper time, and `Transfer`, `Rendezvous` and `Consort` are built on it. `Escort` is the
  exception: its push follows the quarry's acceleration, for as long as the quarry burns, which
  no plan bounds. Its approach is committed as usual and its station-keeping is charged as it
  goes, at the magnitude of the quarry's acceleration — an overstatement, never an
  understatement. The server breaks an escort off when its storage runs out.
- `Burn` changes velocity instantly: Δη is the rapidity of the new velocity relative to the old.
- **Refunds.** `CutDrive`, `BreakOff`, and a standing intercept being re-solved all refund the
  part of the commitment not yet flown, and a re-solved plan commits afresh. A re-solve the ship
  cannot pay for breaks off.
- Burn cost is priced at the mass when the account was last settled. The account settles at least
  once a game day — see [20-solar-power.md](20-solar-power.md) — so what the drain takes off the
  mass meanwhile is priced in a day late, and what that over-commits comes back when the plan
  ends.

### The budget sets the speed

With `F` joules not already committed, the most rapidity a ship can buy is

**Δη_max = −ε · ln(1 − F / (m c²))**,

which is `ε · ln(wet / dry)` for a full ship. A crossing spends it on the match, then splits the
rest evenly between boost and brake. Rather than refusing an unaffordable crossing, **the server
lowers its speed cap**, bisecting it a fixed forty times against the plan's own cost, and returns the lowered `max_beta` in `Accepted`, as
it already returns a lowered acceleration.

That puts a ceiling on the game. A ship that is nothing but storage modules has
`wet/dry = 1 + storage_per_module`, so no ship crosses faster than
**tanh(ε · ln(1 + storage_per_module) / 2)**. Storage capacity and ε are the two numbers that
balance travel.

## Refits

A refit takes the ship from its current loadout to a target one. The client proposes a target,
and both ends run the same planner over the same starting state. That is the same "send the
order, not the trajectory" rule [08-networking.md](08-networking.md) applies to courses.

### The planner

`Refit::plan(loadout, target, energy, balance)` returns an ordered list of steps, or refuses and
names what is short. Each step is one module built or dismantled, or the hull grown or shrunk by
one slot. It is greedy:

1. **Build drones first**, when there is room and energy. Every later step is faster for it.
2. **Grow the hull** when the target needs more slots than it has.
3. **Build** the next module when there is a free slot and the energy for it.
4. Otherwise **dismantle** the next module the target does not want. Living, data and engines
   go before storage. Whatever of its 95% refund the storage left afterwards has no room for is
   **thrown away**, as is what a full storage module held; `Refit::vented_j` says how much.
5. **Shrink the hull** once enough slots are free.
6. **Dismantle drones last.**

The two orders are `BUILD_ORDER` and `DISMANTLE_ORDER`. A target that wants a module built or
taken apart that its order has no place for is refused naming it (`CannotBuild`,
`CannotDismantle`) rather than falling through to a shortage it is not short of.

If no step is possible, the target is refused with the reason. **A target must keep at least one
drone**, since dismantling the last one would leave nothing to finish the job.

### Timing

A step moves `E` joules at `drones × drone_power_w`, so it takes `E / (drones × drone_power_w)`
seconds and the whole refit is a closed form in time. A data module is the exception: it takes
`data_work_factor` times as long as any other module, whatever it costs. Energy moves continuously through a step, so
stored energy has no jumps. The server folds completed steps as the clock passes them.

Steps and the living drain are both measured in **coordinate** time. A refit only runs when the
ship is not under way, where the two clocks agree to parts in a billion, and a drain that read the
crew's clock would need the motive to evaluate.

### Cancel

Completed steps stay. The step in progress is reversed, and whatever of its energy had already
moved comes back at the 95% dismantling rate. A dismantling's refund so far goes back into the
module, less anything already thrown away, which stays gone. The ship is left in the partial loadout, which may be
worse than either end, and that is intended.

### Flying and refitting exclude each other

- `Order::Refit` is refused while `ShipState::is_under_way()`.
- Every order that lights the drive is refused while a refit is running.

A ship holding station, falling or drifting may refit. A ship hanging about beside another
(`Consort`) is under way, and has to break off first.

## Stored energy

`stored_j` at `at_t`, and a closed form from there:

- minus living drain, **clamped at zero** — running out has no consequence yet
- minus burn spending, which comes out of what was committed rather than what is free
- plus or minus the flow of any refit in progress
- never above `storage modules × storage_per_module × ME`

Free energy is stored minus the commitment still outstanding. The drain draws only on free energy.

## Balance

One struct, `lc_world::fitting::Balance`, with a `DEFAULT`. The server holds one
(`Server::set_balance`) and **states it with every account** in `Outbound::Fitted`, for the reason
`rate` is stated with the clock: a client assuming a constant would confidently preview refits
against numbers the server does not use. Stating it beside the account rather than in `Welcome`
keeps the welcome's shape and puts the numbers next to what they govern.

| setting | default | meaning |
|---|---|---|
| `drive_efficiency` | 1.0 | ε. Unbounded above |
| `recovery` | 0.95 | fraction of build energy a dismantling returns |
| `storage_per_module` | 5 ME | capacity of one storage module |
| `engine_thrust_n` | 7.24 × 10¹⁰ N | 1 g of the starting ship, full, per engine, so five engines give today's 5 g |
| `drone_power_w` | 2.31 × 10¹⁹ W | one drone builds one module in a week of proper time — about 69 s of real time at the design rate |
| `living_drain_w` | 4.43 × 10¹⁵ W | one living module drains 1 ME a century |
| `data_mass_fraction` | 0.5 | a data module's mass, and so its build energy, over any other module's |
| `data_work_factor` | 3 | how many times longer a data module takes to build or take apart than any other |
| `data_per_module` | 2.1 MB | a year of a thirty-minute stare in every band: a surveyed sky's files fit several times over, and raw logs are what fill it. A craft with none still has a 1 MiB onboard store |
| `transmit_gain` | *anchored* | physical link-budget energy to stored energy; see [23-factions.md](23-factions.md#cost) |
| `hull_density` | 50 kg/m³ | frame mass per slot, and so what growing the hull costs |
| `slot_volume_m3` | 392 699 | |
| `module_density` | 395.8 kg/m³ | |

### What the defaults give

The starting ship: 5 engines, 6 storage, 2 drones, 1 living, 1 data, 5 slots empty, 20 slots, storage full.

| | |
|---|---|
| dry / wet mass | 2.65 × 10⁹ / 7.31 × 10⁹ kg |
| stored | 30 ME |
| acceleration, full / empty | 5 g / 13.8 g |
| Δη for a full tank | 1.02 |
| fastest crossing | 0.47c |
| 1 AU in-system at 5 g | 1.3 days, 0.84 ME |
| fastest any ship can cross | 0.71c |

| `storage_per_module` | ε | starting ship's crossing | fastest possible |
|---|---|---|---|
| 2 | 1 | 0.26c | 0.50c |
| 5 | 1 | 0.46c | 0.71c |
| 10 | 1 | 0.63c | 0.83c |
| 10 | 2 | 0.90c | 0.98c |

## Protocol

`PROTOCOL_VERSION` 19 → 20, and new golden bytes. **New variants are appended**, so every existing
discriminant keeps its encoding.

- `Order::Refit { target: Loadout }` and `Order::CancelRefit`
- `Outbound::Fitted { ship_id, fitting }`: the balance, the loadout, stored energy at `since_s`,
  the motive's rapidity then, the outstanding commitment, and the refit under way as its recipe.
  Sent after `Welcome`, after every accepted order and every `Flying`, when a refit finishes, and
  after a grant. The client takes it whole.
- `Inbound::Grant { joules }`: **development only**. Honored on a server that is directing, as
  `Inbound::Stage` is, and on a shard only from an **admin**: an account whose ticket carries
  `perm` of at least 1. See [16-identity.md](16-identity.md).
- `Refusal::NoEnergy`, `Refusal::NoRoom`, `Refusal::Refitting`, `Refusal::UnderWay`
- `Order::SetCourse` and `Order::Cross` gain `max_beta`: asked for by the client, and returned in
  `Accepted` lowered to what the ship could pay for.

## Persistence

`SAVE_FORMAT` 3 → 4. `Saved` gains the loadout, stored energy and its timestamp, the
outstanding commitment, and any refit in progress. Rows in format 2 or 3 load with the starting
loadout and full storage.

A ship loaded from an old row is 3.8 times heavier than it was, so its acceleration ceiling is
unchanged — that is how `engine_thrust_n` was chosen — but its plume, which reads mass, gets
brighter. The plume keeps the exhaust-power formula for now; passing it `mass_kg_at(t)` is one line
when the visuals should follow.

## Client

- **HUD**: stored / capacity, and the commitment while one is outstanding.
- **Refit panel** (`Panel::Refit`, key `R`, `--panel refit`):
  - a slider per module kind, and one for slots. They move over everything a hull could hold;
    the planner alone judges whether the draft can be reached, and Apply says why not. A looser
    budget check once bounded them instead, disagreed with the planner, and pinned sliders
    with no reason given
  - the preview, from the planner run locally: energy available (stored + refunds − builds −
    hull), capacity after, step count, duration, and whether it can be done
  - **Apply**, disabled *with the reason shown* when it cannot be done or the ship is under way
    ([18-ui-style.md](18-ui-style.md)). The draft survives Apply, so a refused refit leaves the
    sliders where they were; **Cancel** while a refit runs, with progress
  - a warning, not a refusal, when the plan would throw energy away for want of room in storage
- **Dev actions panel** (`Panel::DevActions`, key `F5`, `--panel dev`): *+1 ME*, *+10 ME*, *fill
  storage*. Development only, and a shard takes them only from an admin.
- **Flight panel**: the acceleration buttons offer what the ship is rated for now, not fixed
  values up to `MAX_ACCEL_G`.

Every button emits an `Action`, and the preview is a pure function of `Session` and `Ui`, so it is
tested without a window ([13-client-shell.md](13-client-shell.md)).

## Where it goes

| crate | new | changed |
|---|---|---|
| `lc-world` | `fitting.rs` (loadout, balance, mass, rating), `cost.rs` (Δη per plan, the rocket law, budget), `refit.rs` (planner, timing, cancel) | `craft.rs` gains the loadout and ledger and derives length from slots; lit-time helpers on each plan |
| `lc-proto` | | the variants above; version and goldens |
| `lc-server` | `fitting.rs`, since `server.rs` is already near 3000 lines | `act()` commits and refunds, and uses the rated drive instead of `kind.drive()`; the tick folds refit steps; pursuit re-solves pay; `persist.rs` format 4 |
| `lc-client` | `refit_panel.rs`, `dev_panel.rs` | `ui.rs`, `action.rs`, `hud.rs`, `uplink.rs`, `session.rs`, the flight panel |

## Order of work

As planned, and done in this order.

1. **`fitting`**. Tests: the 500 m hull is exactly 20 slots; derived length round-trips; the
   starting ship full is 5 g; stored energy weighs `E/c²`.
2. **`cost`**. Tests:
   - a burn cut into `n` pieces costs what the whole costs
   - low speed: cost ≈ `m Δv c / ε`
   - `Δη_max` spends exactly the free energy
   - `mass_kg_at` agrees with the formula part-way through a crossing
   - ε > 1 is cheaper and nothing divides by it badly
3. **`refit`**. Tests: short of energy or slots, it dismantles before building; no refund
   overflows storage; drones come first and go last; an impossible target names its shortage;
   the timing adds up; cancel keeps what finished and reverses what did not.
4. **Craft integration**: the rated drive replaces `kind.drive()` for fitted ships; mass follows
   the ledger.
5. **Protocol**, then **server**, then **persistence**. Break each mechanism on purpose and check
   the test fails, per [AGENTS.md](../../AGENTS.md): an order refused for energy, a refit refused
   under way, a grant refused by a shard that is not directing, a format 3 row loading.
6. **Client** panels and HUD, photographed with `--panel refit --shot`.
7. Update [03-world-model.md](03-world-model.md) *Ships* and *Resources* and
   [13-client-shell.md](13-client-shell.md) *Windows*; mark this doc built. Run
   `tools/api_surface.py` on every crate touched, then `cargo clean`.

## Open

- **Balance is a guess.** Every default above is a first number to argue with once it can be flown.
- **Zero energy** has no consequence. Living space is where one would go.
- **Energy income** — collectors, per [03-world-model.md](03-world-model.md) — is what makes
  the dev grant unnecessary. The first source, hull solar, is designed in
  [20-solar-power.md](20-solar-power.md).
- **Transmission** should draw on the same budget; `Order::Transmit` states a power and is free.
- **Other modules**: weapons, cargo, sensors. The planner's order of priority will need a rule
  for each.
- **A full ship throws energy away** when it takes a module apart, rather than being refused.
  The refit panel warns how much, and a player who cares makes room in storage first.
- **Holding a station is free**, as it was before energy: `motion::thrust_g` treats the
  milligravities as zero, and so does the cost.
