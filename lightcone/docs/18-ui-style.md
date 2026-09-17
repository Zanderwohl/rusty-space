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
the scene. See [19-library.md](19-library.md).

What the exception does **not** licence is hiding the world behind it. The flight readout and the
staleness figure stay visible, and notifications still draw on top; the whole premise of the
feature is that the player is waiting for something, so the thing they are waiting for has to be
able to interrupt them.

## The palette crosses the boundary, not the guess

One source — `em_ui::vfd` — and a conversion at the edge:

```rust
fn colour(from: bevy::prelude::Color) -> egui::Color32
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
- **Colour is never the only signal.** A disabled button is disabled *and* says why it cannot be
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
