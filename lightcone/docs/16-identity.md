# Identity

Who a player is, and how a socket proves it. One service, owned by neither product, that the
website and the game both consume and neither implements.

[14-hosting.md](14-hosting.md) deferred this and named the two places it slots in. This is what
slots in.

## The service is a broker, not a login page

It is an OAuth2 **client** facing upstream and an OAuth2 **issuer** facing downstream. Those are
different jobs and conflating them is how an auth service ends up owning passwords.

```
Google / Discord / itch.io        the site        the game client
        (upstream IdPs)           (confidential)    (public)
              |                        |                 |
              |  authorization code    |  code + PKCE    |  ticket
              v                        v                 v
        +---------------------------------------------------+
        |                   lc-identity                     |
        |  accounts | links | sessions | signing keys        |
        +---------------------------------------------------+
```

**It never stores a password.** Every account arrives through an upstream provider, and the
`links` table is the "where did this account come from" the broker exists to answer: one row per
`(account, provider, provider_subject)`. A player who signs in with Discord today and Google
tomorrow gets one account and two links, and the game never learns that either happened.

Email is a link like any other, and a magic link is a provider. That keeps "sign in with an
email" from being a special case with its own credential handling.

## Three databases, one opaque id

| owns | knows | never sees |
|---|---|---|
| `lc-identity` | accounts, links, sessions, keys | anything about the world or the site |
| game server | `account_id` → craft, progress | email, provider, display name history |
| site | content, releases, subscribers | craft, positions, play history |

The only string that crosses is an **opaque account id** — a UUID that means nothing outside the
broker. No foreign keys span the three, because they are three deploy units on three schedules
and a join across them is a coupling that outlives whoever wrote it. This is the same rule
[14-hosting.md](14-hosting.md) already applies to the build id.

The game additionally receives a **display name** and nothing else. It is a cache, not a fact:
the broker owns the name, the game shows whatever it was told at connect, and a rename appears
next session. A game that treats a display name as an identifier has to handle a rename, which
is the bug this avoids rather than solves.

## The game does not run an OAuth dance

The obvious design makes the game client a public OAuth2 client and has it do
authorization-code-with-PKCE like everything else. Do not. The browser build is served from the
**CDN origin**, which is not the site origin, so that path needs a redirect URI on a host that
serves static files, cross-origin token storage, and a refresh flow in wasm. All of that to
re-establish something the player already has: a session on the website.

Instead, `/play` mints a **game ticket**.

```
player is signed in to the site (ordinary code+PKCE, confidential client, session cookie)
  -> GET /play
  -> site asks lc-identity for a ticket, scoped to the game server audience
  -> ticket is handed to the client in the page that launches it
  -> client presents it as the first thing on the socket
```

A ticket is a signed JWT with a deliberately hostile shape:

- **audience** the game server, and only it. A ticket is useless anywhere else.
- **lifetime** sixty seconds. It is carried from a page load to a socket open, which is one
  round trip, not a session.
- **single use.** The game server records the `jti` until it expires. A replayed ticket is
  refused, so a ticket in a log or a screenshot is worth nothing a minute later.
- **claims** `sub` (account id), `name`, `iat`, `exp`, `aud`, `jti`. Nothing else. A claim the
  game does not need is a claim that leaks.

The native client cannot rely on a website session, so it runs the full code+PKCE flow against
the broker with a loopback redirect, holds a refresh token in the OS keychain, and exchanges it
for the same kind of ticket. **Both clients present the same thing to the game server**, which
therefore has exactly one code path and no notion of which build it is talking to.

## Signed, not introspected

The game server verifies a ticket **locally**, against a public key fetched from the broker's
JWKS and cached. It does not call the broker per connection.

The tradeoff is revocation latency, and it is the right way round here: a sixty-second ticket
*is* the revocation window. The alternative — an opaque token the game introspects on every
open — makes the broker a hard dependency of the game's liveness, so that an auth outage stops
existing players from reconnecting. A game whose sessions are hours long and whose world keeps
running cannot afford that.

Key rotation is the ordinary JWKS one: two keys published, sign with the newer, verify with
either, retire the older a ticket-lifetime after it stops being used.

## What a socket must do

Today `Inbound::Hello { protocol }` carries no identity, so every connection is anonymous and
`admit` hands out a craft to anyone who asks. That is the hole this closes.

```
Hello { protocol, ticket }      first message, or the socket is closed
  -> verify signature, audience, expiry, and that jti is unseen
  -> look up the craft that account owns, or create one
  -> Welcome { client_id, ship_id, now_t, name }
```

Rules that matter more than they look:

- **The client never names its own ship.** `ship_id` comes back in `Welcome` and is looked up
  from `sub`. A client that could ask for a `ShipId` could ask for someone else's.
- **Nothing before `Hello`.** Any other message on an unauthenticated socket closes it. Not a
  refusal — a close, because there is no one to refuse.
- **A deadline.** A socket that has not said `Hello` within a couple of seconds is closed. An
  idle unauthenticated socket costs a file descriptor and proves nothing.
- **A pre-auth budget.** [`rate`](../../crates/lc-server/src/rate.rs) meters per `ClientId`,
  which does not exist yet at this point. Unauthenticated connections need their own budget,
  keyed by address, and it should be much tighter than the authenticated one: the only legal
  message is `Hello`.
- **Identity is fixed at open.** The ticket is not re-presented and the session does not expire
  mid-flight. A session has a maximum length and ends by the socket closing, after which the
  client gets a fresh ticket the same way it got the first one. Tearing down a player's
  connection because a token aged out is a worse failure than a long session.

`ResumeFrom { arrive_t }` becomes reachable only after `Hello`, and the range it asks for needs
a cap: a reconnecting client asking to be replayed from the beginning of time is a resource
attack whether or not it means to be.

## What this does not do yet

Named so they are decisions rather than omissions:

- **Entitlements.** Whether an account may play at all, or on which shard, is a game question
  and belongs in the game's database keyed by account id. The broker answers who, not what.
- **Bans.** A ban is the game refusing a valid identity, which is the right layering — the
  broker should not learn what a player did in a world it knows nothing about.
- **Multiple characters.** [09-open-questions.md](09-open-questions.md) has not settled whether
  a player ever inhabits more than one ship. `Welcome` returning a single `ship_id` assumes not;
  if that changes it becomes a list and the client picks, which is a protocol change and a
  version bump.
- **Account deletion.** Deleting the broker row orphans the game's rows rather than cascading,
  because a cascade across three databases is the coupling this design spent its whole budget
  avoiding. The game reaps orphans on its own schedule.

## Where it lives

Its own cargo workspace, beside `web/`, for the reason `web/` has one: a second lockfile is what
makes "deploys on its own schedule" a mechanism rather than an intention. It shares no code with
either product and must not appear in either dependency tree.

```bash
cargo tree -p lc-server | grep -i identity     # must be empty
```

The game server depends on a **public key and a JWT library**, not on the broker.
