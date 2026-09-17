# The client shell

The application around the simulation: states, menus, windows and the rules they answer to.
`bevy_egui` for everything dense with text. The exception is the main menu, which composites
over a rendered background and is Bevy UI — see [The main menu](#the-main-menu).

[18-ui-style.md](18-ui-style.md) is the companion: this page is what the interface *is*, that
one is how to draw a surface without it fighting what is already there.

## States

```
AppState     Boot -> MainMenu -> Loading -> InGame                 desktop
             Boot -> Loading -> InGame, and Unreachable from either  browser
MenuPage     sub of MainMenu:  Root | NewWorld | Load | Settings | About
Overlay      sub of InGame:    None | Escape | Settings | Debug
```

**Nothing is modal.** Since the clock never stops, an overlay that blocks the world behind it
is claiming something untrue. `Overlay` is therefore not a state at all — the escape menu, the
settings screen and the debug window are panels in the same set as the telescope, each toggled
independently. What remains of `AppState` is where the application is, not what is on top of it.

`MainMenu` is the desktop entry point and a thin one. **The browser build has none**, gated by
`app::HAS_MAIN_MENU`: the page that launched it has already signed the player in and named the
shard, so it opens straight into `Loading`, and its escape panel has no Quit, because there is
no menu to return to and no process to end.

Without a menu there is nowhere to fall back to, so a browser build that cannot reach its shard
— refused, lost, or never named one — goes to `Unreachable`. It is terminal: it says what went
wrong and to reload the page. Nothing reconnects yet, which is what makes that honest.

`Settings` appears under both parents and is one screen either way. It is reachable from the
main menu before a world exists and from the escape overlay while one is running, so it may
not assume a world.

`Loading` exists because it has work to do: the catalogue is 120 000 rows and generation is
per star. A frozen window is not a loading screen.

## The game does not pause

**Coordinate time advances regardless of UI state.** Opening the escape overlay, the settings
screen or the debug window does not stop the clock, and neither does losing window focus.

This is an invariant with a test, not a convention, because two things will try to undo it.
Gating the simulation schedule on `in_state(Overlay::None)` is the idiomatic Bevy spelling and
costs one line. And a single-player instinct says a menu should pause. Once a server owns time,
anything that learned to pause is broken, and it will have learned quietly.

Two consequences to design for now:

- **Settings apply live.** There is no "apply on resume", because there is no resume.
- **The overlay says so.** It dims the world and leaves the clock ticking in the corner, which
  teaches the rule better than a tooltip.

## The main menu

Bevy UI, not egui: it sits in front of a starfield and has to composite with it, and there is
no point drawing an opaque egui panel over a sky in order to hide the sky. The widgets come
from `em-ui`, shared with Exotic Matters, so both products' menus are one implementation in
two palettes.

It emits actions like every other surface: a button writes `Requested(Action)` and the
dispatcher does the rest. Nothing in the menu mutates state directly, and the page it draws is
read from `UiState::menu_page` rather than mirrored into a Bevy state, so there is one record
of where the player is.

### The backdrop is generated, not loaded

The sky behind the menu is drawn by the real starfield pass, at rest, but its stars are not the
catalogue's. Loading the catalogue is what `Loading` exists for — a hundred and twenty thousand
rows, and generation per star — and a menu that waits for it is the frozen window that state
was added to avoid.

So the menu generates four thousand main-sequence stars from a fixed seed: isotropic, uniform
in volume out to four hundred light-years, masses from an inverted Salpeter IMF and effective
temperature from the same mass so the colours agree with the sizes. Fixed rather than random
per launch, because a backdrop that differs each time reads as a bug in the sky.

The field turns at one radian a minute, in yaw only. `Look` carries no roll by construction, and
a horizon that rotates is a worse backdrop than one that pans. The heading the drift reaches is
where the ship starts looking; there is no better default.

`--menu --shot <path>` photographs it, because a window nobody is watching proves nothing.

## The staleness readout is the premise

Always on screen, never in a panel a player can close:

| readout | example |
|---|---|
| coordinate time | `T + 14.62 years` |
| age of the selected target's light | `Proxima Centauri — light is 4.24 years old` |
| what the band mapping is | `NATURAL` |

If a player forgets they are looking at the past, the game has failed at the only thing it is
about. This belongs where a shooter puts the health bar.

## Instruments are not settings

Some controls change *what a player can know*, and those are gameplay, not preferences:

| control | binding | why it is not in Settings |
|---|---|---|
| band mapping preset | `1`-`4` | see [07-rendering.md](07-rendering.md); the composition preset is a diagnostic |
| exposure, stops from auto | `[` / `]` | the tone map has no absolute reference, so exposure is a camera control |
| telescope target and integration | panel | the core loop |
| scale tier | `Tab`, and automatic by distance | the three tiers behave differently enough to be worth showing |

Burying the band presets in a settings menu would be burying the instrument the game is played
with.

## Windows

| window | opened by | contents |
|---|---|---|
| telescope | `T` | target, band, exposure time, survey regime, the light curve, uncertainty |
| system | `Y` | bodies and populations of the selected system, at the retarded time |
| sky | always | the all-sky map; selection happens here |
| notifications | automatic | target out of range, observation returned nothing, instrument saturated |
| radio | `R` | one conversation at a time, chosen from a list of everyone heard from and everyone in sight |
| debug | `F3` | below |
| scenarios | — | scenes to stage. Development only, and every button does nothing without a shard started for it |

Panels are windows rather than menu pages because the clock never stops: a player has to be
able to watch a curve and fly at the same time.

### The radio window

**One window and one selector, not a thread per craft.** Everything in it is minutes to years
old and there is no typing indicator to be had, so the interface that suits it is a log with a
dropdown rather than a messaging app pretending the far end is present. A message someone sent
shows a dot until they acknowledge it, and then it shows `ack` — which is the only delivery
report there is, for the reason in
[05-observation.md](05-observation.md#acknowledgement-is-the-only-delivery-report).

A transmission arriving is also a **green line in the notifications box**, and that line is a
link. It is the one kind of event with somewhere to go: everything else in that box is the
interface reporting on itself. Somebody else's sealed message is a line too, saying that it was
heard and cannot be read — a signal falling on the antenna is a fact about the world, and hiding
it would let a player learn that nothing was sent by not being told.

Sealing is offered only for a craft whose key this ship holds, and the checkbox says why when it
is not. The client's copy of that rule is an interface courtesy; the server refuses the order
either way.

**The radio window is why `read_keys` consults egui.** Every binding in the table below is a
bare letter, and nothing in the game had a text field until there was something to say into one
— so typing a message used to open the telescope, cut the drive and fly somewhere, one keystroke
at a time. Held arrow keys are gated the same way: an arrow in a text field moves the cursor, and
turning the ship as well would make going back to fix a typo swing the whole view.

## The debug window

Development only. Its God view entry is compiled out of shipped builds and gated server-side
besides, per [07-rendering.md](07-rendering.md).

| entry | why |
|---|---|
| God view toggle | `#[cfg(feature = "godview")]` |
| causality overlay | lines from in-flight events to the observers they are travelling toward. The one bug class — an observer learning early or late — that no other view shows |
| **time-rate multiplier** | a transit at 8766x takes real hours. Dev only: the server owns the rate, and a client that can change it is a client that can cheat |
| scale tier and camera distance | the three-tier reduction is invisible until it is wrong |
| retarded-solver statistics | roots found, Newton iterations, bracket expansions |
| write snapshot | the phase 6 headless renderer, from inside the running app |
| frame and entity counts | ordinary |

## Settings

| group | contents |
|---|---|
| display | resolution, vsync, tone-map window width, default band preset |
| controls | sensitivity, invert, rebinding |
| accessibility | UI scale, redundant encodings — see below |
| audio | later |

Settings persist to a small file in the user config directory. Settings that do not survive a
restart are a development annoyance that becomes a shipping bug.

## Accessibility is load-bearing here

The band mapping is an **information channel**, not decoration. The composition preset works by
showing dust as orange and a solid occulter as neutral, which is the diagnostic
[05-observation.md](05-observation.md) and phase 3 built the observation model around. A player
who cannot separate those two colours cannot play that part of the game.

**Every colour-carried readout gets a numeric twin.** The deficit ratio between two bands is a
number as well as a hue; a star's temperature is a number as well as a tint. This is cheap now
and structural later.

## Every action is a message

**No UI system acts directly.** A key press, a button, a menu item and a test all produce the
same `Action` value, and one dispatcher applies it. Nothing else changes `UiState` or reaches
into the session.

```
Action        an enum: ToggleP anel, SetBandPreset, ExposureUp, SelectTarget, Quit, ...
apply()       Action + UiState + Session -> Vec<Effect>
Effect        the few things needing the engine: Quit, WriteSnapshot, Notify
input.rs      the only place that knows about KeyCode; maps input to Action
```

Rebinding is not built yet and this is what makes it a table rather than a rewrite. It also
buys the things that usually arrive too late to be cheap: driving the UI from a test with no
window, replaying a session from a list of actions, macros, and a remote control for
debugging. The cost is one enum and one match.

`Action` is engine-free. Only `input.rs` mentions `KeyCode`, so the core stays testable and a
second front end — a touch build, a script — needs no new plumbing.

## Structure

UI state is data and the systems are thin, for the reason
[06-crate-layout.md](06-crate-layout.md) records: a renderer made of ECS systems cannot be
reused, and neither can a UI made of them be tested.

```
UiState        open panels, selected target, exposure offset, preset index
action::apply  the only thing that mutates UiState
hud::lines()   what the readout says, as plain strings
panels::*      egui systems that draw UiState and emit Actions
```

Anything that decides *what* to show is a function over `Session` and `UiState`, testable with
no window — which is the only way any of it gets verified, since a window cannot be inspected
from a test.

`lc-client` does not share the Exotic Matters `src/gui/style.rs`. Phase 5 established that the
GUI does not extract, and the two products want different looks.

## Running it

```bash
cargo run -p lc-client --bin lightcone -- assets/catalogs/hygdata_v42.csv
```

Omit the path for the three authored sample stars.

| key | does |
|---|---|
| `Esc` | close the top panel, then the menu |
| `T` `Y` `F` `F3` `F4` | telescope, system, flight, debug, starfield tuning |
| arrows, right-drag | look |
| | the cursor is pinned while the right button is held, and released on let go |
| `L` | look at the selection |
| `G` `X` | cross to the selection, cut the engine |
| `N` | target the nearest system |
| `1`–`6` | band presets |
| `[` `]` `\` | exposure down, up, auto |
| `,` `.` | clock rate down, up along the ladder |

The clock rate is a development control and the server owns it in a real session. It is
labelled by period rather than by factor — `1 year / 10 s`, not `360x` — because a factor is
not something anyone can feel, and the head-up display flags any rate off the design one so a
fast clock never looks normal.

**Owning it means stating it.** The rate is in the welcome and is restated with every clock,
and a joined client runs at whatever it is told. It used to hold a constant of its own that
happened to agree, which is a different thing: a shard at any other rate would have been joined
by a client confidently running at this one. What made the difference worth paying for is that
a scene can now choose its clock — a low orbit of Jupiter comes round every four hours, which
at the design rate is a revolution every second and a half, and a three-month chase at a year
a minute is fifteen seconds of watching.

One consequence that is not obvious and cost a debugging session to see coming. The slack the
client's clock is corrected against has to scale with the rate. It was a fixed coordinate hour,
which is comfortably more than a statement's own age at the design rate and is *less than one
tick* at sixty — so every statement would look like a runaway and the client would snap back
seven hours twenty times a second having never drifted at all.

### Development flags

| flag | does |
|---|---|
| `--observe` | skip the menu and start in the sky |
| `--shot <path>` | photograph the sky through the real pipeline, then quit |
| `--fly` | cross to the nearest interstellar star |
| `--band <n>` | band preset |
| `--rate <n>` | clock multiplier against one year per hour: `360` is a year per ten seconds |
| `--watch` | target the nearest system and open the telescope |
| `--swarm` | target the nearest star carrying a swarm |
| `--curve <n>` | which band the light curve measures |
| `--tune` | open the starfield tuning panel |
| `--frames <n>` | frames before the shutter |
| `--at <body>` | stand off a named body of the local system |
| `--station <course>` | put the ship straight on a station: `orbit:Earth`, `polar:Mars:high`, `rings:Saturn`, `l2:Earth`, `belt:0`, `leave` |
| `--panel <name>` | open a panel by name |
| `--burst <n>` | photograph `n` consecutive frames, numbered. For flicker: two runs stopped at frame `n` and frame `n+1` have accumulated different wall time and are not consecutive at all |
| `--demo <name>` | stage a scene, and bring a shard to run it in |
| `--demo-cam <yaw:pitch:booms>` | pin the camera for the run |
| `--say <words>` | say this to the first contact that appears, and open the conversation. The only way to photograph a transcript |

These emit [`Action`]s rather than opening a second path into the client, so they can only do
what the interface can do. `--shot` exists because WGSL cannot be asserted from a test and a
window nobody is watching proves nothing; both images in
[07-rendering.md](07-rendering.md) were taken with it. `--rate` without `--shot` is the screen
recording setup.

`--demo` is the exception that proves the rule, and it is worth saying why. It cannot be an
action, because what it asks for is a craft placed somewhere by fiat and that is the one thing
no client may ask for — every order on the wire is a request a ship makes about itself. So it
is a message of its own, honoured only by a shard started for it, and the scene it names is
staged on the side that decides what happened. See `lc_world::scenario` for the scenes and
`lc_server::director` for the runner.

What the camera does during one is the ordinary free orbit: a scene is something to look
around inside, not something to be shown. It turns once to face everything the camera is *not*
on — by the middle of them by bearing, so four hulls spread about an axis are framed down the
axis rather than at whichever is nearest — and is the player's from then on. `--demo-cam` pins
it instead, for a frame two runs are meant to agree about.

Which craft the camera is behind is a `CameraPerspective`, and it has one variant. An enum
anyway, because what the camera does is going to grow — a chase view along the velocity, a fixed
point a scene is composed from, a free fly-around — and each of those is a different answer to
"where is the eye" rather than a flag on top of this one.

**A scene says where to stand, and how fast to run.** Both are fields on it rather than things
to pass in: it knows what it is about, and a rendezvous is two different events seen from its
two ends. `approach` and `closing` are the same two ships doing the same manoeuvre, watched from
one end and then the other.

**The eye moves and the observer does not**, which is the boundary to know about. Everything the
client works out about light — retarded times, aberration, what a contact looked like when it
left — is still solved from the player's own ship, because that is the craft the session has a
worldline for. Across a scene, where the cast is kilometres apart, the difference is
microseconds and there is nothing to see. Across the Oort cloud it would be hours. Watching from
a craft you are not on is a development view until the observer can move too, which is why the
only way to reach it is a flag and a panel that does nothing in a shipped build.

## Getting about inside a system

![High orbit of Earth](../images/orbit.png)

![In the asteroid belt](../images/belt-station.png)

Five things a ship can be told to do — cross to a point, take up an orbit, sit at a libration
point, drop into a belt or a ring, leave the system — are two mechanisms, not
five. A **waypoint** says where to be at any coordinate time; the same crossing that goes
between stars takes the ship there. Arriving is not the end of it: the ship then *holds* that
waypoint, which is what makes an orbit an orbit rather than a point it drifts away from.

A hold is read, never integrated. The waypoint is a closed form in the coordinate time, so a
paused clock, a clock at a year a second and a dropped frame all leave the ship in the same
place, and nothing accumulates.

None of this is orbital mechanics, and the ship never transfers. It is a torch under five
gravities: it points at where the destination will be and burns. What the simulated mechanics
are for is the *destinations* — a real Hill radius for the libration points, a real ring plane
out of the body's own pole, a belt at the radius that actually carries its light.

Three things the shapes buy:

- **An altitude is in radii above the surface**, not kilometres, so `low` means the same thing
  at Deimos and at Jupiter — bodies four orders apart in size.
- **A polar orbit is one whose normal is perpendicular to the body's pole**, and an equatorial
  one has the pole for its normal. One line either way, and the same `Orbit` draws a ring
  system by taking the ring plane instead.
- **A crossing meets an orbit at its nearest point.** An orbit is a circle and a ship arriving
  at one has a near side; entering at whatever point the clock happened to have it can mean a
  crossing straight through the body. The phase is therefore chosen when the course is planned
  rather than when it is resolved — which point is nearest is a question about where the ship
  is coming *from*, and that is not known until then. It is part of the same fixed point as the
  arrival time, since the nearest point moves while the ship is flying.
- **Leaving goes straight out from the star**, along the radius the ship is already on. Not
  along the star's own axis: that is one fixed direction whatever the ship is doing, so every
  departure rose out of the ecliptic instead of taking the shortest way out. A ship at the star
  itself has no radius to follow and falls back to the axis.
- **A crossing leads its target.** Flip-and-burn time goes as the square root of distance, so
  aiming at where the body will be converges in three rounds. It matters: Earth runs a
  fiftieth of an astronomical unit during a crossing from Mars, which is four thousand
  planetary radii of miss.

### The System window

![The System window](../images/system-window.png)

Two sections, and nothing flies between them.

The **inventory** runs outward from the star with each body's satellites behind it. The
ordering is hierarchical rather than by distance from the star, because a moon's heliocentric
distance *is* its planet's: ordering on that would shuffle Jupiter's moons into whatever
arrangement they happened to be in this instant. Each body is keyed by the chain of orbital
radii from the primary down to itself, so planets come out by their own distance and a planet's
moons come out behind them by theirs. Bands land among the planets by radius, which puts the
asteroid belt between Mars and Jupiter where it belongs.

The whole thing is built **once, when the system loads**, from the positions at one epoch. A
list that reorders itself while a player is reading it is worse than one that is a few per cent
stale. Straight out of the file every body sits at the origin and has no parent — the derived
columns are rebuilt by the first evaluation — so the first version of this came out in file
order with every radius zero.

Forty-one of the solar system's bodies are the ones it is usually described by and a hundred
and eighty-nine are not, so the list shows the former until you ask for `all`.

Picking one opens its **courses**: equatorial and polar orbits at three altitudes, the two
collinear libration points if it has a parent, above the rings if it has rings, and leaving the
system if it is the star. What is offered is what exists — a moon of nothing has no libration
points, and rather than grey the option out it is not there. A test flies every option every
major body offers and fails if any of them fails to resolve, so the list cannot lie.

Arming a course and flying it are separate: **Go** is what commits, and it uses the ship's own
acceleration. The crossing is a brachistochrone to the injection point and then the ship holds
station on it.

**A body is called what it is called.** Its own name first, then whatever catalogue designation
it carries, and only then a made-up one — the primary's name and a Roman numeral, which is how
an unnamed body has been designated since Galileo. When players can name worlds, that name goes
in the first slot and nothing else changes.

### Cutting the engine does not stop the ship

**Decided: cancelling keeps the velocity, and inside a system that velocity is an orbit.**

The `×` beside the flight readout cuts the drive, with no confirmation — the action is not
destructive and a dialogue between a player and their own throttle is worse than the mistake it
prevents. What it does is stop the *engine*. The ship keeps what it had, and what it had is now
a conic about whichever body's sphere of influence it is in.

None of that arithmetic is written here. `em-foundations` turns a state vector into elements
and back and solves both anomalies; `em-sim` finds the sphere of influence. The client is the
join, plus the patched-conic rule: one conic is exact only inside one sphere of influence, so
when the ship crosses into another body's the arc is re-solved about it. Done when the crossing
is reached rather than predicted ahead, which is the cheap half of patched conics and the half
that cannot be wrong.

A torch ship makes this less forgiving than it sounds. Five gravities passes solar escape
velocity in minutes, so cutting out of a brachistochrone halfway to Earth leaves an eccentricity
of a hundred and sixty — a near-straight line out of the system. Cancelling off a station gives
back the orbit the station was holding, which is the case the readout is really for.

Two things it cannot do:

- **A radial state has no elements.** A ship at rest relative to what holds it is falling
  straight down the line to it: the angular momentum is zero and the orbital plane is
  undefined, so no conic can be written. It refuses, and a refusal leaves the ship drifting at
  the velocity it has, which is none. Standing still is the wrong physics and the right
  behaviour; falling into the star over the following two months is neither.
- **Parabolic is nudged off.** Both anomaly solvers divide by the distance from `e = 1`, and a
  state landing exactly there is an accident of arithmetic rather than a trajectory anyone
  chose.

There are now three ways for the ship to be moving and they are exclusive: under thrust the
crossing says where it is, on station the waypoint does, otherwise it is ballistic — a conic
inside a system and a straight line between them. All three are **read** at the new time rather
than integrated from the old one, so a paused clock, a clock at a year a second and a dropped
frame all leave the ship in the same place. Holding a station used to be a rendering system
running a frame behind the model; now that the session owns the local system it is one branch
of three in the same place.

### The clock has to slow down for an orbit

The design rate is already 8766 times real time, which puts a whole low orbit inside a second.
The rate ladder therefore reaches below the design rate as well as above it — real time, a
minute a second, an hour a second — and the navigation panel prints the orbital period next to
the station and says so when the clock is outrunning it.

### A ring has no thickness, so there is nowhere inside it to stand

![On station at Saturn's rings](../images/ring-station.png)

The ring station started out mid-annulus, in the ring plane — which is what "enter a ring"
sounds like, and it was unusable. A ring is drawn as a surface with no thickness. With the ship
inside the annulus that surface passes through the camera: the nearest geometry is at no
distance at all, and the camera's own offset from the plane is smaller than f32 render
positions can hold — about six metres at Saturn. The rings swung between a hairline and a
bright wedge from one frame to the next, and nine and a half per cent of the screen changed
every frame at real time.

A ring station now stands a quarter outside the outer edge, in a plane tipped a quarter turn
out of theirs, so the rings read as rings and the orbit still closes them to a line twice a
turn. Outside the annulus the same six metres of jitter is six metres in a hundred and forty
thousand kilometres, and the flicker falls by a factor of four thousand — from 87,704 changed
pixels a frame to twenty, which is what a star field crossing pixel boundaries costs anyway.

Edge-on from *outside* still shimmers on the one-pixel line the rings collapse to. That is
ordinary geometric aliasing of a thin bright edge, it is bounded, and it is 141 pixels.

### The near plane is fifteen metres

A low orbit is a fraction of a planetary radius above the surface, and the render unit is an
astronomical unit. The camera's near plane stood at a million and a half metres, which is
further from Earth than a low orbit is: the sphere was clipped away entirely while its own
billboard still drew, so a planet filling the sky rendered as a dot. Reversed float depth costs
nothing for the range — its precision is relative — but the constant the star field writes is
tied to it, because that has to stay under the smallest depth a real body produces.

## The light curve

![The light curve](../images/light-curve.png)

Drawn by em-plot into a `tiny_skia` pixmap and handed to egui as a texture, rasterised only when
what it shows changes. Not with egui's own painter: em-plot already has the min/max column
decimation that a curve of thousands of samples in a panel of hundreds of pixels needs, and a
second plotting implementation is the thing to avoid.

Two rules the picture follows:

- **It plots relative flux, not the deficit.** With re-emission a measurement lands *above* the
  unobscured star, and a plot of how much is missing cannot show that at all.
- **The baseline is always in frame.** A curve auto-scaled to its own noise looks like a
  detection when the star is doing nothing.

The caption is egui text rather than pixels. Baked into the bitmap it collided with the axis
labels, and egui draws text better than a twelve-pixel bitmap font.

The band the curve measures is chosen separately from the display mapping. They are different
questions — one is what the instrument integrates, the other is how three numbers become a
colour — and the screenshot above is the reason they have to be separate: the display is in the
thermal preset and the curve is in the thermal *band*, and only the second one is what makes the
excess a number.

A band the sensor cannot reach is shown as unavailable rather than omitted, so the instrument's
limits are visible instead of merely enforced.

## Tuning the starfield

`F4` opens every drawing parameter of both starfield passes as a slider, applied as it is
dragged. The panel is generated from a table beside the parameters themselves, so a knob cannot
be added without a slider appearing for it, and a test checks that the table covers the struct
and that every shipped value falls inside the range its slider offers.

It follows the same rule as everything else: the panel reads state and emits actions, and a
drag is one `SetPointStyle` per frame carrying the whole style. Nothing in the panel mutates,
so the same tuning can be driven from a test or a script.

Uniforms are written only when a value actually differs from the last upload. Reaching for
`get_mut` on a Bevy asset marks it changed whether or not anything did, and re-uploads the
buffer; at rest the starfield now uploads nothing at all.

Nothing here is saved. These are numbers to find, not settings to keep — once one is right it
belongs in `DISTANT` or `LOCAL` where it can be read and reasoned about.

## Not in this phase

WASM, networking, and God view in any shipped build. The last is compiled out from the start
rather than added and later removed.

## Open

- Whether the sky map is a window or the world seen from a ship. Likely both, in the manner of
  Space Engine: a view through a camera, and a three-dimensional stellar map centred on the
  observer. The flat map is the cheaper first version and the two want different camera code.
