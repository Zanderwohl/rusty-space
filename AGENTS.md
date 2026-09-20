# Working in this repository

Read [CLAUDE.md](CLAUDE.md) first — layout, conventions, the comment policy and the file-size
cap live there and are not repeated here. This file is the rest: how to check your work, and
the things that have already cost someone a day.

Design documents are the readable form of the argument and are kept current:
`lightcone/docs/` for the game, `docs/` for Exotic Matters. Read the one for the area you are
touching before changing it, and update it when the answer changes.

## Housekeeping

- **`cargo clean` when you finish.** `target/` reaches **70 GB** in this workspace. Nothing
  warns you.
- **Never `--release`.** Dependencies are already `opt-level = 3` in the dev profile; a release
  build costs many minutes of link time for a speedup you will not notice.
- **Commit promptly.** Worktrees get recycled and uncommitted work is gone for good.

## Two invariants, both enforced by CI

```bash
cargo tree -p em-foundations | grep -i bevy     # must be empty
cargo tree -p exotic-matters | grep -E '^\s*lc-' # must be empty
```

`em-foundations` is engine-free. Exotic Matters and Lightcone are two products on the same
shared crates (`em-*`); Lightcone's own crates (`lc-*`) may never be reached from the app.

## Checking your work

`python3 tools/api_surface.py crates/<name>` prints a crate's public surface and its line
counts. Run it at the end of a piece of work; the printout is the review artifact.

**WGSL cannot be asserted from a test, and a window nobody is watching proves nothing.** The
client photographs itself through the real pipeline:

```bash
cargo run -p lc-client --bin lightcone -- assets/catalogs/hygdata_v42.csv \
    --station rings:Saturn --panel system --rate 0 --shot /tmp/shot.png --frames 90
```

| flag | for |
|---|---|
| `--shot <path> --frames <n>` | photograph and quit |
| `--burst <n>` | photograph `n` **consecutive** frames — the only way to see a flicker |
| `--at <body>` / `--station <course>` | stand off a body, or start on a station |
| `--lift <deg>` | raise the ship out of the ecliptic about the star, keeping its distance |
| `--panel <name>` / `--tune` | open a panel |
| `--book <id>` | open a book from `crates/lc-client/assets/books/<id>.epub`; `--chapter <n>` and `--pages <n>` move within it |
| `--menu` | hold at the main menu, so `--shot` photographs that instead of the sky |
| `--signin` | hold at the sign-in modal, which draws over the menu and no action can reach |
| `--password` | hold at the password form, the one egui surface inside the menu |
| `--turn <deg>` / `--pitch <deg>` | turn the view, the only way to put something off screen |
| `--zoom <notches>` | move the orbit camera; both its stops are clamps, so ask for far too much |
| `--map <bearing:elevation:au>` | pin the map's camera. A pin, so two shots of it are the same shot |
| `--map-plane <ecliptic\|galactic>` | which plane the map lays its rings in |
| `--demo <name>` | stage a scene: `traffic`, `meeting`, `approach`, `closing`, `chase`. Brings its own shard |
| `--demo-cam <yaw:pitch:booms>` | pin the camera for the run, so two shots of a scene are the same shot |
| `--rate <n>` | clock multiplier; `0` freezes it, which makes frames comparable. Offline only — a shard states its own. **The default is the design rate**, so a run without this flag is as slow as the game |

`--turn`, `--pitch` and `--zoom` are applied **last**, after anything that aims — `--fly` ends
by pointing the view at what it is flying to, and pushed first the turn was simply undone.

They are still *actions*, though, so they race whatever else aims the camera; the note further
down about a pitch that did not land three runs in a row is about exactly that. `--demo-cam`
does not race anything, because it is written every frame after the dispatcher rather than once
on arrival. Prefer it for anything two runs are meant to agree about.

A scene replaces what used to be `--traffic <n>` and `--chase`, and does more than either: the
craft have names, sizes and somewhere to be, and `lc_world::scenario` is where what they do is
written. `--demo traffic` is the old fan of hulls. The clock is the scene's to state — `meeting`
and `approach` run at a twentieth of the design rate because a low orbit otherwise sweeps the
whole view past twice a second, `closing` at a tenth, and `chase` at twenty, which is where
three months of running becomes about a minute of watching.

Most of what has gone wrong in the renderer was found this way and could not have been found
any other way.

`--lift` is there because every station is in the ecliptic and every population's pole is the
ecliptic pole, so without it a belt is edge-on in every photograph that can be taken and a
shell is a band. It leaves the ship ballistic rather than holding — the waypoint would put it
straight back in the plane — so pair it with `--rate 0`.

The store's tests need PostgreSQL (`createdb lc_store`; `LC_STORE_URL` overrides). They
**skip** when they cannot reach one — keep it that way, so the suite passes without it.

## Traps

Each of these cost real time. None of them are visible from the code that hits them.

**The light-cone model**

- A motive is a **closed form total in `t`**, which means it cheerfully answers about times
  before it was ever flown. `Craft` keeps a history of the stretches it has flown for exactly
  this reason — without one, changing a motive rewrites the craft's whole past, and every
  retarded solve reads the new motion at the old time. That shipped once and leaked every
  maneuvere instantly to every client in the system. Change a motive only through
  `Craft`'s own methods; they are what record it.
- `Cleared::clear` gates **when** a message may be sent and says nothing about how its content
  was computed. A message can pass the gate and still be a fact from the future.
- A test that asserts "X did not happen early" passes trivially if X never happens at all, or
  if it happens for an unrelated reason. Break the mechanism on purpose and check the test
  fails — both of the light-delay tests in `lc-server` were wrong the first time, and both
  looked right.

**Rendering**

- Projecting a *path* by projecting each point and dropping the ones behind the camera draws a
  **chord**: the two survivors either side of the gap get joined, and a ring seen from inside it
  acquires a straight line across the view that no ring has. Cut the segments at the camera
  plane instead — `em_ui::reticle::project_path`.
- `Camera::world_to_viewport` **errors** for anything behind the camera, so nothing built on it
  can point at what is behind you. Work in clip space and keep `w`: `clip.w` is `-view.z`, so
  behind the camera it is negative while `clip.x` keeps the sign of `view.x`. Dividing anyway
  mirrors the point through the center. See `em_ui::reticle::place`.

- Depth is **reversed**. `clip.z = clip.w` is the *near* plane. Background geometry wants a
  tiny positive value, not zero — the buffer clears to zero and the test is strictly greater.
- `AlphaMode::Add` is *premultiplied*: `src + dst*(1-alpha)`. For pure additive the fragment
  must return **alpha 0**, or it overwrites and two coplanar meshes flicker on sort order.
- Render positions are f32 relative to the camera: about **six meters** at a hundred thousand
  kilometers. Never place the camera on a surface — an infinitely thin sheet containing the
  camera swings wildly from frame to frame.
- Two runs stopped at frame `n` and frame `n+1` are **not** consecutive frames. They have
  accumulated different wall time. Use `--burst`.
- `--turn` and `--pitch` **do not always land**. Three runs of one `--pitch 60 --frames 200`
  gave two frames at pitch zero and one pitched. Two runs of the same command are not
  byte-identical either, so a hash tells you nothing. Check that the shot is the view you asked
  for before you measure it, or compare against something in the frame that cannot move.
- The window is **not always the same size**. A screenshot taken on one display and one taken
  after the laptop moved to another are 1280x720 and 2560x1440, and a patch measured at fixed
  pixel coordinates then samples two different parts of the picture. It reads exactly like a
  regression and is not one. Measure in fractions of the frame.
- **A second camera turns every `.single()` camera query into an early return.** Seven systems
  in `lc-client` wanted "the camera"; adding the map's did not draw a wrong picture, it drew no
  picture — the view froze, the stars sized to zero and nothing was pickable, with nothing in the
  build to say why. `SkyCamera` is the disambiguator. `bevy_egui` has the same shape of problem
  one layer up: it gives its primary context to the **first camera created**, and two `Startup`
  systems have no order between them, so the whole interface went into a 512-pixel texture.
- **A render target that will be resized needs `COPY_SRC`.** `Image::new_target_texture` sets
  three usages and not that one, and `Image::resize` copies the old contents forward. The first
  resize is a wgpu validation failure, and it takes the application down long after the frame
  that caused it. `RenderTarget` is also a *component* in Bevy 0.19, not a field on `Camera`;
  left off, the camera clears the primary window to black.
- **A line is a tube, so it has a width the geometry does not know about.** Two consequences,
  both found by looking. A camera closer to a plane than a tube's angular radius is *inside* the
  nearest ring, and the inside of a tube is a solid wall — an edge-on map came out as a
  rectangle of flat green. And a tube's thickness has to be set by its **near** end: every point
  of a ring is equidistant from the center, but a spoke runs from the eye to the rim, and a
  width that is a pixel at the far end is eighty at the near one.
- WGSL reserves more words than you expect. `from` and `target` are both reserved and both are
  natural names in a ray marcher; the error arrives from the pipeline cache at run time, not
  from `cargo build`.

**`em-sim` and `em-foundations`**

- Derived columns — `parent`, `position`, `mu` — are empty until the first propagation.
  Reading them straight after `System::from_contents` gives zeros and no hierarchy.
- `System::mu(i)` is the `mu` of the orbit body `i` is *on*, i.e. `G(M_parent + M_i)`. To orbit
  *around* `i`, use `gravitational_constant() * mass(i)`.
- `Instant::to_j2000_seconds()`, not `seconds_since_j2000()`.
- The arena holds **one instant**. `System::position(i)`, `LocalSystem::body_position_ly` and
  friends read it; `propagate::state_at` and `LocalSystem::body_state_at` answer for any time
  without touching it. Mixing the two — a craft read at `t`, the body it orbits read out of the
  arena — turns a circular orbit into a wild ellipse. Nothing in `lc-world` or `lc-client`
  propagates any more; if you need a position, say which instant you mean.
- **A planet's frame is not inertial.** Earth turns eight degrees in nine days, so a "straight
  line past Earth" posed at J2000 is a curve by the time it arrives, and a flyby slower than
  30 km/s is Earth running into the craft rather than the reverse. Any test that predicts a
  chord, a miss distance or an impact angle has to be fast enough that the frame holds still —
  see `FLYBY_SPEED` in `em_sim::collision`.
- Sphere-of-influence radii scale with the **live** separation, so they breathe over an
  eccentric year: Earth's L2 is 1.476 million km at J2000 (near perihelion) and 1.501 at the
  mean distance. A published figure is the mean one.
- At a patched-conic join the craft is *exactly* on a boundary, so `influence::containing` is a
  coin toss and it comes up "the sphere you are leaving". Take the new primary from the
  crossing, as `em_sim::patch` does.

**Editing by script**

- `str.replace` in a Python one-liner **fails silently** when the pattern is absent, and
  `cargo fmt` reflowing a match arm is enough to make it absent. Two edits were lost that way
  and the code still compiled, because the thing they set had a `Default`. Assert the text
  changed (`assert s != before`) or grep for the result afterwards — a compile is not evidence
  the edit landed.

**Formatting**

- **`cargo fmt` is not run on the game workspace.** CI fmt-checks `auth/` and `web/` only, and
  the game's code is hand-formatted — `cargo fmt --all` at the repository root rewrites 204
  files and 22 000 lines, burying a change in churn. Format the files you write to match their
  neighbors and leave the rest alone.

**The administration console**

- `auth/lc-admin` **runs no migrations.** `lc-identity` owns every one of them and applies them
  at its own boot; the console reads and writes tables it did not create. Start the broker first
  or the console comes up against a schema that is not there.
- Its TypeScript is compiled by a **stage of the container build**, not by cargo. `cargo build`
  succeeds without it and the *boot* fails, with a message naming `npm run build` — which is
  where you will meet it, because `Assets::load` reads `static/js/admin.js` off disk.
- **htmx 4, not 2.** Attributes no longer inherit implicitly (`hx-target:inherited`), events are
  colon-separated (`htmx:after:swap`), a GET does **not** send its enclosing form's values
  (`hx-include="this"` on the filter form is what makes the filters work), and every status but
  204 and 304 is swapped — which is why a refusal returns 422 with a body rather than being
  dropped. `npx htmx.org upgrade-check` catches htmx 2 habits.
- **Three htmx mistakes here failed silently and looked entirely correct.** All three were found
  by driving a browser and none by a test:
  - `hx-trigger="input changed ..."` on a `<form>` **never fires**. `changed` compares the value
    of the element the trigger is on and a form has no value. The search box did nothing.
  - `target:(#q)` — the parenthesised selector form the documentation gives for selectors
    *containing whitespace* — matches nothing; the parentheses are not stripped. `target:#q`
    works.
  - `htmx:after:swap`'s `event.target` is the element that **issued** the request, not the one
    that was replaced. The swapped element is `event.detail.ctx.target`. Keying on `event.target`
    type-checks and quietly skips every swap that came from a form.

  The lesson is the one the renderer section already draws: a page nobody has opened proves
  nothing. Serve the console, set the session cookie by hand, and click.
- A paged query without a **unique tie-break** in its `order by` shows a row on two pages and
  another on none. `Listing::order_by` appends `a.id` for this, and a test asserts it for every
  column.
- **The console's tests share one database and never drop it.** An assertion that reads page
  one of an unfiltered index passes on a fresh database and starts failing once enough runs
  have accumulated to fill a page — which looks like a regression in the thing it is named
  after and is not. Narrow to accounts the test made, by a name carrying a uuid, as
  `the_pages_partition_the_matches` does.
- **A refusal page needs a link out.** The console has no navigation except a masthead that
  renders for administrators, so a refused visitor sees a page with nothing on it to click and
  no way to guess the address of anything — including `/signout`, which existed the whole time
  and is a **POST**: `SameSite=Lax` sends the session on a cross-site top-level navigation when
  the method is safe, and never on a cross-site POST, so the method is the whole of the
  defense. A link to it would not work and is asserted against.
  `views::refusal` takes a way out; `views::wrong` is for store failures, where there is
  nothing useful to offer.
- **A session that can only be refused should not exist.** Check the level before sealing one,
  and clear it on the path that refuses an existing one. Otherwise the two combine into a
  cookie its holder cannot get rid of.
- htmx is **vendored**, and `npm run build` refuses when the committed copy is not the one
  `package-lock.json` pins. To take a new htmx: bump the dependency, `npm run vendor`, commit
  both.

**axum**

- An array of header pairs in a response **inserts**, which replaces any header of the same
  name. Two `Set-Cookie` entries therefore leave one — the last — and a sign-in that sets a
  session and clears a nonce silently drops the session. Use `AppendHeaders`.
- A static path beats `{param}` in the router, so `/signin/password` and `/signin/{provider}`
  coexist. They are only reached by different methods here, which is worth keeping true.

**Stylesheets**

- `_tokens.scss` exists **three times**: the site's is the original, and the broker and the
  administration console each hold a copy. Both copies are compared to the site's byte for byte
  by a `the_tokens_are_the_sites_tokens` test. If one fails, a file was edited — copy the site's
  over it rather than making them "close enough". They are copies because the three are separate
  docker build contexts and the file cannot be shared. Service-only additions go elsewhere:
  `auth/lc-identity/static/styles/_status.scss` for the broker,
  `auth/lc-admin/static/styles/_base.scss` for the console.
- The broker compiles its sheet in `build.rs`, so a SCSS error is a failed build. The site
  compiles at boot and needs `cargo run --bin lc-web -- --check-styles` to catch one earlier.
  They are different on purpose; `lightcone/docs/16-identity.md` says why.
- **Chrome paints an autofilled input itself** and ignores `background` and `color` outright,
  which on a dark panel is a pale box with invisible text. Only a tall inset
  `-webkit-box-shadow` plus `-webkit-text-fill-color` reaches it. A sign-in form is the one
  place this always shows up.

**OAuth2, upstream**

- The ID token from a token-endpoint exchange is **not** signature-checked, deliberately: it
  arrived over a TLS connection we opened, authenticated with our client secret, so a signature
  proves nothing the transport has not. OIDC Core §3.1.3.7 rule 6. `iss`, `aud` and `exp` are
  still checked, and `lightcone/docs/16-identity.md` has the reasoning. Do not "fix" this by
  adding a JWKS fetch.
- A test that only asserts "the hostile token was refused" passes just as well when every case
  fails for some unrelated fourth reason. Assert the **error kind** per case.
- A stub provider that agrees with whatever it is sent proves nothing about PKCE. Make it store
  the challenge at `/authorize` and compare `S256(verifier)` at `/token` — then check the test
  actually fails when the verifier is wrong, because a stub asserting nothing looks identical
  to a stub asserting everything until you break the code on purpose.

**egui**

- Interface rules live in `lightcone/docs/18-ui-style.md`: which toolkit a surface belongs to,
  one surface at a time, and why anything over another panel is opaque.
- **Bevy UI is retained**: a surface rebuilt every frame loses `Interaction`, so its buttons
  never show a hover. Key the rebuild on *what is drawn*, not on `Res::is_changed` — the menu
  backdrop writes `ResMut<Ui>` every frame, so that flag is always true.
- **Bevy UI orders by spawn**, so two systems spawning into one frame have no order between
  them — an overlay drawn by one lands *behind* the screen drawn by the other, interleaved with
  it. `em_ui::MenuUi::overlay` sets a `GlobalZIndex` for this reason. And a translucent panel
  over another of the same size reads as one muddled thing: a modal's panel wants full alpha.
- An overlay on `Order::Background` is painted *under* every panel and floating area, so the
  interface covers it. `Order::Foreground` is over all of them — keep such an overlay inside
  `ctx.available_rect()` so it does not draw on top of a docked panel.
- The default font has no U+2715 `✕` — it renders as a tofu box. U+00D7 `×` is fine.
- `add_enabled` wrapping a `SelectableLabel` reports clicks nobody made. A plain
  `selectable_label` does not.

**PostgreSQL**

- `numeric ^ 2` is exact but goes through `numeric_power`, which picks a display scale: `-1`
  comes back as `-1.0000000000000000`. Multiply instead — scale zero, and cheaper.
- Migrations need a **session advisory lock**. Without one, two processes both find the step
  table missing, both create it, and one dies on a duplicate key in `pg_type`.
- An index-only scan still checks the heap for every row until a **vacuum** marks pages
  all-visible, and the planner correctly prices it as no better than a bitmap scan until then.
  A plan that should be index-only and is not usually means no vacuum has run.
- `tokio-postgres` has no `jsonb` conversion for a Rust string. Send `text[]` and cast in the
  statement.

## When a test disagrees with the code

Assume the test premise is wrong about as often as the code is — most of the disagreements in
this repository so far have been the assertion, not the implementation. Numbers written from
memory (an orbital period, a threshold, a formula) are the usual culprit.

The exception is worth knowing: if you write a brute-force reference to check a fast path,
**derive it independently**. A reference written from the same mistaken idea as the thing it
checks will agree with it and prove nothing. One here did not agree only because the two
filtered their results differently, which is luck.
