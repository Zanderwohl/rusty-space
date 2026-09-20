# Interface style

How a surface is built, which toolkit it belongs to, and what it may do to the one behind it.

[13-client-shell.md](13-client-shell.md) is what the interface *is* — states, windows, and why
the game never pauses. This is how to draw one without it fighting what is already there. Every
rule below was learned by photographing something that looked wrong.

`em-ui` is shared with Exotic Matters, so the **mechanisms** here apply to both products and the
**palette** does not: each names its own and passes it in.

## Two toolkits, and the line between them

| | |
|---|---|
| **egui** | anything dense with text, and anything needing input |
| **Bevy UI** | the menu, which composites over a rendered sky |

The second is the exception and stays small. The first is the default because the game is mostly
readouts, and because **Bevy UI has no text input at all** — a form is an egui surface or it is
a custom widget nobody asked for.

A single egui surface *inside* a Bevy UI screen is allowed, and the password form is one. What
it costs is a palette that has to be carried across the boundary by hand; see below.

### A rendered image inside an egui surface

A third case beside the two above, and it behaves because of one decision: the image is
allocated with an explicit `Sense` rather than shown with `ui.image`. An interactive allocation
makes egui *want* the pointer, and `crate::input`'s wheel and cursor grab both already stand
down when it does — so a drag on the map does not also fly the ship, with no new coordination
and no flag.

Which button matters. **Right** turns the map, because right is the sky's look button and the
map is the other mode of the same screen: one button meaning opposite things on the two of them
is worse than either meaning. So a right-press that starts on a map surface turns the map and
not the sky behind it. The grab is asked for once, on the press, and `grab_cursor` stands down
while egui wants the pointer. That is a drag belonging to the widget it began on, which is
right; what makes it worth saying is that the map's corner square is never closed, so that
corner always answers.

## One surface at a time

When something is modal, **what is behind it stands down**. Not dimmed and still readable —
gone.

This is not taste. A menu behind a modal is a second thing to read and a second set of buttons
to try, and two panels of similar size at the same place read as one muddled object rather than
as one in front of another. The sign-in modal draws and the menu builds nothing; the password
form opens and the modal's panel goes with it. Each keeps its backdrop, so the sky is still
dimmed and the player can see they are somewhere.

The screen node itself is still spawned when its contents are not, because it carries the marker
that says which page is drawn. Losing that turns a rebuild into a flicker.

## Rebuild on what is drawn, never on what changed

Bevy UI is **retained**. A surface that is despawned and respawned loses everything the engine
was holding for it — including `Interaction`, which is how a button knows it is being hovered.
A modal rebuilt every frame has buttons that are destroyed and recreated before the next frame
can notice a cursor on them, so they simply never light up. Nothing looks broken; the hover is
just absent.

So the trigger is a comparison against **what is currently drawn**, not a change-detection flag:

```rust
#[derive(Component, PartialEq)]
struct Modal(Shown);          // what this surface is showing

if already_drawn == wanted { return; }
```

`Res::is_changed` is the trap, and it is a good trap because it looks exactly right. The menu's
backdrop drifts by writing `ResMut<Ui>` every frame, so `Ui::is_changed()` is *always* true —
measured, 121 frames out of 121 — and a modal keyed on it rebuilt 121 times. Keyed on its
contents it builds once.

None of this applies to egui, which is immediate-mode and has no retained state to lose. It is a
rule about the retained toolkit only, which is part of why the line between them is worth
keeping sharp.

## Z-order is not implicit

**Bevy UI orders by spawn.** Two systems spawning into the same frame have no order between
them, so an overlay drawn by one lands *behind* a screen drawn by the other — interleaved with
it, text through text. `em_ui::MenuUi::overlay` sets a `GlobalZIndex` for exactly this, and an
overlay is above by definition rather than by luck.

The map's corner square is `Order::Middle` for the same reason read the other way: it has to go
*under* an open window rather than over it. The map itself, when it is the whole view, is in
`Order::Background` — which is what makes every window float over it.

**egui has its own orders**, and they are not the same thing. Panels paint in
`Order::Background`, floating windows and notices in `Order::Middle`, and an overlay that wants
to be over both goes in `Order::Foreground`. A mark on the wrong one vanishes under whatever the
interface happens to have open, which is how the picking reticle's edge arrows first disappeared
behind a notice box.

## Opacity says what a thing is over

An ordinary panel is translucent — 92% — because it sits over the rendered world and letting a
little of it through is what keeps the panel feeling like part of the scene.

**Anything over another panel is opaque.** Two translucent surfaces at the same place blend into
one, and the reader gets both sets of text at once. The sign-in modal's panel is opaque; so is
the password form's frame. Both were translucent first, and both looked broken in exactly the
same way.

**And a page of prose is opaque and light**, which is the one surface in the client that does not
take the palette at all. `crate::reader` draws black serif on a gentle white, inside a dark case,
because a player should know what it is before reading a word of it — and because prose over a
drifting starfield is unreadable in a way a readout over one is not. The rule's own reason argues
for the exception: a panel is translucent so it feels part of the scene, and a book is not part of
the scene. See [21-library.md](21-library.md).

What the exception does **not** license is hiding the world behind it. The flight readout and the
staleness figure stay visible, and notifications still draw on top; the whole premise of the
feature is that the player is waiting for something, so the thing they are waiting for has to be
able to interrupt them.

## The palette crosses the boundary, not the guess

One source — `em_ui::vfd` — and a conversion at the edge:

```rust
fn color(from: bevy::prelude::Color) -> egui::Color32
```

An egui surface inside the menu dresses itself from that, so the two agree by construction. The
alternative is a hex value typed into egui that drifts the first time the palette moves, and
nobody notices because it is only *nearly* right.

What an egui surface has to set, because its defaults are its own:

- `override_text_color`
- `widgets.inactive.bg_fill`, `widgets.hovered.bg_fill`, `widgets.active.bg_fill`
- `extreme_bg_color` — what a text field is filled with. Darker than the panel, or an empty
  field is invisible and the form reads as labels with no inputs.

Game panels drawn over the sky keep egui's own dark theme and that is fine: they sit on a
rendered background, not inside the menu, and matching the menu there would be matching nothing.

## Three faces, and where each one stops

The interface is set in **Quantico**: every readout, label, button and window title, on both
toolkits' surfaces. It is a squared technical sans, which is what a panel of numbers over a
rendered sky should look like, and it is the default — a surface that wants a different face has
to say why.

Both toolkits, and they get there differently. egui is told once, through
`faces::Faces::install`, and names the family by string. Bevy UI has no font set to name a
family in, so it takes a `Handle<Font>` per label, which is what `MenuUi::font` is for and why
`faces::UI_FILE` is public: the menu and the sign-in modal load their own copy of the same file,
and the one constant is what stops the two toolkits landing on different cuts of it.

Two surfaces say why they are not Quantico.

**The title screen is Nabla**, and only the title screen. The game's name is a wordmark and the
site sets it in the same face, so the menu and the front page are recognizably one thing. Nabla
is a color font whose depth lives inside the glyph: it is unreadable at the size a heading is
set at, so `MenuUi::title_font` takes a size with the handle and the menu asks for 44. Bevy
flattens its layers into one color, which on the VFD palette is exactly the extruded green a
title wants. It costs 1.6 MB and is asked for on the one screen that draws it.

A `title` with no wordmark falls back to `MenuUi::font` at the ordinary heading size, which is
what the sign-in modal wants: its heading is "SIGN IN", not the product's name, and the
wordmark face would be claiming otherwise.

**A radio log is Geo**, and only a radio log — the speaker's name and what was said, in
[`radio_panel`](../../crates/lc-client/src/radio_panel.rs). Not the list of craft, not the
composer, not the markers this client adds to a line: those are the ship talking to its pilot,
and the log is another ship talking to this one. It is the same distinction the one green in the
palette already makes, said a second way.

Geo needs a fifth more point size than Quantico to read at the same size beside it — its
capitals fill 0.56 of the em against Quantico's 0.70 — which is what `LOG_SCALE` is.

The reader is not an exception to any of this; it is not this interface. It is dressed as a
thing you hold and sets its own page, its own headings and its own folio in its own two faces.
See [21-library.md](21-library.md).

### Hinting off, and what that is worth

egui rounds glyph coordinates to the pixel grid by default. macOS has not hinted since
CoreText dropped it, so a face chosen by looking at it in a browser was chosen unhinted, and
`faces::unhint` turns it off to match.

It is worth knowing how little that buys, so nobody measures it again: hinting is a program
carried in the font, and **Quantico and Faustina carry none** — no `fpgm`, no `cvt`, a
seven-byte `prep` stub. Geo carries a token one. Turning it off is pixel-for-pixel identical on
every surface those three set. What it does reach is egui's own Ubuntu-Light, which is properly
hinted and which sits under the interface family as the fallback for every glyph Quantico
lacks — so the value is that a fallback glyph is not the one crunchy word in an unhinted line.

Two things about where it has to go. egui reads the flag off the **global** style, so the
reader cannot have an answer of its own; and it reads it when a face is *constructed*, not per
pass, so it must be set before anything calls `set_fonts`. Hence a system of its own, ahead of
`faces::settle`.

If the interface ever wants to look heavier or lighter against the same background, the dial is
`TextOptions::color_transfer_function`, not this. egui picks a dark-mode curve
(`TwoCoverageMinusCoverageSq`) from the theme, which is right for light-on-dark panels and
slightly heavy for the reader's black-on-cream page — and being global, that is a trade rather
than a fix.

### One font set, one owner

egui holds a single `FontDefinitions` and `set_fonts` replaces it whole, so two places
installing a face means the second silently undoes the first. Everything that adds one goes
through `faces::Faces::install`, which keeps the accumulated set and hands egui all of it — the
interface faces at startup, the reader's five when a book is first opened.

A named family falls back through the interface family, so a glyph the face lacks is drawn
rather than boxed. That matters: the tofu box has already cost this interface a close button and
a pair of arrows. The reader's faces are the one exception and are installed alone, because a
word in Quantico in the middle of a paragraph of Faustina is worse than a missing glyph.

## Everything is a message

A button emits an [`Action`](13-client-shell.md#every-action-is-a-message). No surface mutates
state directly, including this one — the sign-in modal's buttons emit like any other, and the
part with a socket in it reads the same stream everything else does.

The exception is a form, which owns the text being typed until it is submitted. Even then, what
it does on submit is start a task and set one field, not reach into the world.

## What the player is told

- **One refusal for two causes**, where telling them apart tells an attacker something. The
  sign-in says "that address and password do not match" because saying which half was wrong is
  an account enumerator with a nicer interface.
- **A refusal never names what would have worked.** An error listing valid values is a list of
  targets.
- **Something to act on, always.** The browser sign-in shows the URL as well as opening it,
  because a browser that did not open leaves a player staring at nothing otherwise.
- **Color is never the only signal.** A disabled button is disabled *and* says why it cannot be
  pressed, or is not there.

## Nothing is modal to the clock

The world does not stop behind any of this. A surface that implies it has — a pause overlay, a
"waiting…" that blocks — is claiming something untrue, and
[13-client-shell.md](13-client-shell.md) says why that matters more here than elsewhere. Modal
means *on top of the interface*, never on top of the simulation.

## Photograph it

None of the above can be asserted from a test, and every rule on this page was learned by
looking at a picture and finding it wrong. The client photographs itself:

```bash
cargo run -p lc-client --bin lightcone -- assets/catalogs/hygdata_v42.csv \
    --signin --rate 0 --shot /tmp/shot.png --frames 60
```

`--menu`, `--signin` and `--password` exist because those surfaces draw over things no action
can reach. See [AGENTS.md](../../AGENTS.md) for the rest of the flags.

**And measure rather than assume.** The sign-in backdrop looked absent and was not: the sky
behind it goes from 0.91 to 0.07 mean brightness, thirteen times dimmer. The muddle was two
translucent panels, and "fixing" the backdrop would have made it worse while looking like
progress.
