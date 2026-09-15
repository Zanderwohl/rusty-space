# Reconciliation

What happens when the client's ship and the server's ship are not in the same place.

## This game is not the usual case

The standard client-prediction story is: the client guesses, the server knows better, the client
is corrected. Half of that is wrong here.

**A client cannot be corrected by information that has not reached it.** That is not a
networking constraint, it is the premise — [08-networking.md](08-networking.md)'s gate exists to
guarantee it, and `Cleared::clear` is the only way anything reaches a client. A server that
"corrected" a client using an event still in flight would be transmitting faster than light, and
would delete the game. So the authoritative correction is narrow by construction: **the server is
authoritative about the client's own ship, and about nothing else the client can see.**

The other half is that the fold is deterministic. `lc_world::motion::apply` is the same function
on both sides, over the same events, and a server stepping at 438 seconds a tick and a client at
61 agree *to the bit* — there is a test that asserts exactly that. Two machines running the same
orders do not drift apart for the usual reasons.

So the question is not "how much does the client drift". It is **"what are the specific ways it
can differ at all"**, and there turn out to be four.

## The four divergences

| # | cause | correctable | how |
|---|---|---|---|
| 1 | the client has applied its own intent and the server has not yet | yes, and it resolves itself | wait |
| 2 | the server clamped or refused that intent | yes | re-fold from the acknowledged coordinate |
| 3 | transcendental functions differ between platforms | yes | slew |
| 4 | the client's state is unreachable from the server's | no | replace |

**1 is not an error.** The client applies its own order optimistically at its own coordinate;
the server applies it a fraction of a second later at a coordinate it chose. They converge as
soon as the acknowledgement arrives. This is the common case and it needs no mechanism beyond
knowing which coordinate the server used — which today it does not tell you (see below).

**3 is the one that surprised me.** IEEE-754 pins `+ - * /` exactly, so the same arithmetic gives
the same bits everywhere. It pins **nothing** about `sinh`, `cosh`, `atan2`, `exp` or `sin`,
which are libm's business and differ in the last place between implementations. The fold is full
of them: relativistic flight is hyperbolic, Kepler is trigonometric, and
`anomaly::eccentric_from_mean_newton` iterates until a tolerance — where a last-place difference
in the input can cost an extra iteration and a different answer. A browser's libm is not the
server's. **So a client that did everything right still drifts, slowly, and reconciliation is
not optional.**

The drift is tiny and it accumulates. That shape — small, unattributable, monotonic — is what
the first tier is for.

## Three tiers, chosen by cause first and magnitude second

### Slew — a drift with no event behind it

The authoritative state is adopted **immediately and silently**. What moves smoothly is the
*rendered* position, not the model: the renderer keeps an offset from the true state and decays
it exponentially, time constant around 200 ms, so the picture slides into place over a few
frames while the physics is exact the whole time.

Correcting the model instead of the camera is the classic mistake. A model that is being eased
toward the truth is a model that is wrong on purpose, and everything computed from it — a course
solution, a light-delay schedule — is wrong with it.

**The threshold is angular, not metric.** A tween is worth having when the offset is smaller
than the things on screen, and is worse than a cut when it is not: a slow slide across a
light-second reads as the whole universe moving. So: slew while the offset subtends less than
about a degree at the current view; above that, cut. A metre threshold cannot work in a game
whose scales span from a low orbit to the Oort cloud.

### Adjust — an intent the server did not take as offered

The client re-folds from the last agreed state with what the server actually did. This requires
the server to say what that was, and **it currently does not**:

- `issued_at_client_t` is clamped into `[cursor_t + 1, now_t]` and the chosen value is never
  reported. A client that predicted from its own stamp is predicting from a different event.
- `accel_g` is clamped to the craft's drive, silently. Ask for a thousand g, get five, and be
  told nothing.

Both need an acknowledgement:

```
Accepted { ship_id, event_id, at_t, order }
```

carrying the order **as applied**, not as sent. That closes the loop for 1 and 2 at once: the
client folds `Accepted` exactly as the server folded it, and its optimistic version is discarded
rather than reconciled. The refusal path already exists and is the same shape.

An adjustment is worth *telling the player about*, in words, because they asked for something
and got something else: "acceleration limited to 5 g". It is not a glitch to hide.

### Re-acquire — the state is replaced

A reconnect after a gap, a shard handover, or a server-initiated resync. The client cannot get
from where it thinks it is to where it is by folding anything; it is handed a state and takes it.

This is where a mechanic belongs, and there is a diegetic one sitting right there. The player
sees the world through the ship's instruments, and a replaced state means the ship's own
navigation solution was wrong. So: **the inertial platform lost track and is re-acquiring.**

- the position readout goes to an uncertainty volume rather than a point
- the fix reads as stale, then converges over some seconds
- **and it costs something**: while the fix is unconverged you cannot set a precise course,
  because you do not know where you are

That has teeth, and it is consistent with the premise rather than bolted onto it. Everything you
see has already happened; now so has everything you know about yourself.

## Never turn a bug into a mechanic

The tempting version of the above is "large correction → disorientation", with the size as the
trigger. That is wrong and it is worth saying why.

A large *drift* — tier 3 — is a determinism failure. The fold has a term that depends on the
platform, or worse on the step size, and the correct response is to **log it, alarm on it, and
fix it**. Dressing it up as the ship getting confused hides exactly the signal that would have
found it, and does so most effectively when it is happening most.

So the trigger is **cause**, not magnitude:

- a drift is *always* a slew, however large, and one above a threshold is reported as a fault
- a replacement is *always* a re-acquire, however small

The second half matters too: it means a player on a bad connection, who is only ever in tier 1
and tier 3, is never punished for it. The mechanic fires when the client genuinely does not know
where it is, which is a fact about the fiction and not about their router.

## What has to exist

| | |
|---|---|
| `Outbound::Accepted { ship_id, event_id, at_t, order }` | the order as applied; closes tiers 1 and 2 |
| a client-side event log | the last agreed state plus unacknowledged intents, to re-fold from |
| a render offset with an exponential decay | the slew, in the renderer only |
| a drift meter | offset per unit coordinate time, reported; the alarm for tier 3 |
| `Outbound::Resync { state }` | the explicit replacement, so a re-acquire has a cause and is never inferred |

`Welcome` now carries that state — a `lc_proto::Motion`, which is the whole ship rather than a
point — so the **reconnect** half of the re-acquire exists. What does not yet exist is the
server-*initiated* one: nothing can hand a client a replacement outside a sign-in, so a server
that decides the two have diverged has no way to say so. That is what `Resync` is still for, and
it carries the same `Motion` when it arrives.

Craft are durable. A shard given `--db` checkpoints its whole fleet every twenty real seconds
and on the way out, and reads it back at boot — the clock included, without which every saved
motive, stamped in absolute coordinate time, would read as one that has not happened yet.

The stored form is [`lc_proto::Motion`] in **postcard, not JSON**, and that is not a taste.
`serde_json` does not round-trip every f64: `-1.8149592025296526e-22` comes back
`-1.8149592025296529e-22`, and that value is a real coordinate of a real crossing. A checkpoint
that moved a ship by one place every restart would be a slow leak in the exact property this
whole document is about.

Within one shard's life, though, a signed-out ship keeps flying. The tick advances the whole
fleet and `disconnected` drops only the connection, so a course set before signing out is flown
while signed out, and signing back in finds the ship on station. At 8766 coordinate-seconds a
real second, an hour away is a year of flight. Two tests pin it, because the natural instinct
is to gate the world on somebody watching it.

A crossing **between stars** is the exception to "arrive and be on station": `Change::Cross`
carries no waypoint, because naming a star names a system and not a place inside one. It arrives
at the 63-au standoff, at rest, in the new system — and choosing an orbit there is a second
order.

A protocol version bump, since `Accepted` and `Resync` change the shape of what a client reads.

## Open

- **How large is the libm drift actually?** It has not been measured. A wasm client and a
  native server folding a thousand identical events, differenced, is a half-day experiment and
  it decides the slew's threshold and the drift meter's alarm. Until then the 200 ms and the one
  degree are first numbers, in the sense [09-open-questions.md](09-open-questions.md) means it.
- **Is a fixed-point or software-libm fold worth it?** It would make tier 3 disappear entirely.
  It would also make every trajectory slower and uglier. Measure first.
- **Does a re-acquire have a duration, or a condition?** Seconds is simplest. "Until you have a
  fix from two known bodies" is better, and is a sensor mechanic the game does not have yet.

## The clock is the first thing that drifts

Before any of the above matters, the two ends have to agree about *when* the ship is. They did
not, and the way that showed up was not a clock complaint — it was a ship that **teleported**.

The client runs its own clock between frames, because it draws far faster than anything arrives.
Nothing corrected it, and the rate ladder let a player multiply it by 3600. So the client's
clock ran away, every order it sent came back stamped in its own past, and folding an order from
the past means folding a manoeuvre that has already finished: the ship jumps to its destination.
Meanwhile the server still believed the ship was in transit, so every order about the system the
client thought it had reached was refused.

Two halves, both needed:

- **The server owns the rate.** [13-client-shell.md](13-client-shell.md) already said so — "a
  client that can change it is a client that can cheat" — and the client simply did not enforce
  it. It is also the client that suffers, which is worth saying because it makes the rule easy
  to keep rather than a tax.

  Refusing to let a player *change* it is not enough, and getting only that far was its own
  lesson: the client's **default** is sixty times the server's, so a joined client ran away from
  it at 143.7 coordinate hours a real second with nobody touching a key. Joining adopts the
  server's rate. A multiplier of one is exactly the 8766 the server advances by, and a test pins
  that correspondence rather than leaving it to be remembered in two crates.
- **The server states its clock**, about once a real second, and the client corrects when it is
  more than an hour of coordinate time out. Not every statement: snapping to each one would drag
  the clock backwards by however long that message spent in flight, once a second, forever.

A correction that fires **every second and never fixes anything** is not drift — it is a rate
mismatch, because the client re-diverges as fast as it is pulled back. That is the alarm working
and is worth recognising on sight; the size of the correction names the ratio.

The slack exists for the honest case, which is not cheating: a browser tab in the background has
its frames throttled, so its clock nearly stops while the world does not. It comes back hours
behind and is pulled straight.

A correction moves the **world's** clock and not the crew's. The ship's proper time is however
long they have actually lived through, and no amount of resynchronising un-ages anybody — which
is a distinct method for exactly that reason.

## A crossing starts from whatever the ship is doing

Two things had to be true for "go to that star" to mean anything from a ship that is already
moving, and neither was.

**The speed along the line is carried.** A burn at constant proper acceleration starting at `b0`
*is* the burn from rest entered part-way through — if a ship boosting from rest reaches `b0` at
`t0`, this ship's trajectory is that one's from `t0` onward. So the generalisation is an offset
and every closed form is unchanged, the brake included, since it still ends at rest. Signed, so
a target behind the ship is the same trajectory entered before it turns around.

**The speed across the line is shed, not lost.** A crossing is a straight line and a ship cannot
fly a line it is moving across, so the plan begins with a **match**: a burn in its own direction
that sheds the across-line velocity. It takes real time and covers real ground, and the line is
drawn from where the ship ends up rather than from where it was — which is why a plan's
`from_ly` (where it was ordered) and the start of its line are two different points.

The one approximation, named because it is not visible otherwise: the along-line speed is held
through the match rather than integrated. A rest-frame boost perpendicular to the velocity
leaves the parallel component exactly unchanged, and this thrust is perpendicular to the *line*
rather than to the velocity — so it is exact when the ship is moving purely across the line,
which is the case the match exists for, and the error grows with the along-line speed while the
match itself shortens with it.
