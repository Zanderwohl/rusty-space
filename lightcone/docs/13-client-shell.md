# The client shell

The application around the simulation: states, menus, windows and the rules they answer to.
`bevy_egui` for everything with text in it.

## States

```
AppState     Boot -> MainMenu -> Loading -> InGame
MenuPage     sub of MainMenu:  Root | NewWorld | Load | Settings | About
Overlay      sub of InGame:    None | Escape | Settings | Debug
```

**Nothing is modal.** Since the clock never stops, an overlay that blocks the world behind it
is claiming something untrue. `Overlay` is therefore not a state at all — the escape menu, the
settings screen and the debug window are panels in the same set as the telescope, each toggled
independently. What remains of `AppState` is where the application is, not what is on top of it.

This also fits the browser, where there is barely a main menu to speak of: you arrive already
in the game. `MainMenu` is the desktop entry point and a thin one; the web build can open
straight into `Loading`.

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
| debug | `F3` | below |

Panels are windows rather than menu pages because the clock never stops: a player has to be
able to watch a curve and fly at the same time.

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
| `T` `Y` `F` `F3` | telescope, system, flight, debug |
| arrows, right-drag | look |
| `L` | look at the selection |
| `G` `X` | cross to the selection, cut the drive |
| `1`–`6` | band presets |
| `[` `]` `\` | exposure down, up, auto |
| `,` `.` | clock rate down, up along the ladder |

The clock rate is a development control and the server owns it in a real session. It is
labelled by period rather than by factor — `1 year / 10 s`, not `360x` — because a factor is
not something anyone can feel, and the head-up display flags any rate off the design one so a
fast clock never looks normal.

### Development flags

| flag | does |
|---|---|
| `--observe` | skip the menu and start in the sky |
| `--shot <path>` | photograph the sky through the real pipeline, then quit |
| `--fly` | cross to the nearest interstellar star |
| `--band <n>` | band preset |
| `--rate <n>` | clock multiplier against one year per hour: `360` is a year per ten seconds |
| `--frames <n>` | frames before the shutter |

These emit [`Action`]s rather than opening a second path into the client, so they can only do
what the interface can do. `--shot` exists because WGSL cannot be asserted from a test and a
window nobody is watching proves nothing; both images in
[07-rendering.md](07-rendering.md) were taken with it. `--rate` without `--shot` is the screen
recording setup.

## Not in this phase

WASM, networking, and God view in any shipped build. The last is compiled out from the start
rather than added and later removed.

## Open

- Whether the sky map is a window or the world seen from a ship. Likely both, in the manner of
  Space Engine: a view through a camera, and a three-dimensional stellar map centred on the
  observer. The flat map is the cheaper first version and the two want different camera code.
