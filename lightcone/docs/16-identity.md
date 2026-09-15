# Identity

Who a player is, and how a socket proves it. One service, owned by neither product, that the
website and the game both consume and neither implements.

[14-hosting.md](14-hosting.md) deferred this and named the two places it slots in. This is what
slots in.

## The service is a broker, not a login page

It is an OAuth2 **client** facing upstream and, facing downstream, something much smaller than
an OAuth2 server. Keeping those apart is what leaves the products holding a token and a name
rather than a credential — and keeping the downstream half small is what makes writing this
reasonable rather than reckless. See [Not an authorization server](#not-an-authorization-server).

```
Google / Discord / itch.io   password           the site        the game client
        (upstream IdPs)      (local, opt-in)    (first party)     (public)
              |                    |                  |                 |
              |  authorization code|  argon2id        |  sign-in code   |  ticket
              v                    v                  v                 v
        +------------------------------------------------------------------+
        |                            lc-identity                           |
        |      accounts | links | secrets | sessions | signing keys        |
        +------------------------------------------------------------------+
```

The `links` table is the "where did this account come from" the broker exists to answer: one row
per `(account, provider, provider_subject)`. A player who signs in with Discord today and Google
tomorrow gets one account and two links, and the game never learns that either happened.

**A password is a provider.** It is tempting to say the broker never stores one, and the design
is cleaner if it does not — but a development environment that can only make accounts by talking
to Google is a development environment where nobody makes a hundred accounts. So `password` is a
provider like the others, its credential lives in a `secrets` table keyed by link, and every
path above it is identical to Google's. Whether it survives into production is not decided here;
what is decided is that it costs one provider row and no special case.

## Which providers exist is configuration

```
LC_IDENTITY_PROVIDERS = password,google,discord
```

One list, not a flag per provider. A misspelling in a list of names fails to boot; a
`GOOGLE_ACCOUNTS=ture` is silently off, and an auth service that is silently missing a provider
is an auth service that locks out everyone who used it.

Two rules that make the list honest:

- **A provider in the list must be configured, or the process refuses to start.** Google needs a
  client id and secret; `password` needs nothing, which is exactly why it needs to be named
  explicitly rather than inferred from configuration being present.
- **`password` is off unless listed.** Off by default means shipping without having thought
  about it leaves it off, which is the right direction for the failure to point.

The startup banner names every live provider. "Which providers does production have" should be
answerable from a log line, not from someone's memory.

## Storing a password, given everything that is deferred

Argon2id, a 16-byte random salt per password, and the whole PHC string stored — algorithm,
parameters and salt together — so the cost parameters can be raised later and an old hash
upgraded on the next successful sign-in. A bare digest column cannot be re-tuned without a
password reset for everyone.

Verification is constant-time, and a sign-in for an account that does not exist performs a dummy
verify anyway. Otherwise "no such account" and "wrong password" are distinguishable by a
stopwatch, which turns the login form into an account enumerator.

Deferred, deliberately, and each with a consequence worth stating rather than discovering:

| deferred | consequence |
|---|---|
| email delivery | **no password reset.** A forgotten development password is a deleted row. This alone is why password accounts are not a product feature yet. |
| email verification | a password account's email is **never verified**, which is load-bearing below |
| captcha | registration is an open endpoint when `password` is enabled, which is why it is off by default |
| breach-list checks | a weak password is accepted |

Not deferred, because deferring it would be a hole rather than a gap: **attempts are budgeted**,
per account and per address. Without captcha and without email, an attempt budget is the only
thing standing between a password provider and credential stuffing.

## Aligning accounts by email, and the trap in it

Email is the one thing two providers can agree about, so it is what lets a player who used
Discord in March and Google in April end up with one account instead of two.

**Only a verified email may align anything.** OIDC providers say whether they have verified an
address — Google's `email_verified`, Discord's `verified` — and an address without that claim is
a string someone typed.

The trap is specific and it is created by the two decisions above taken together. Password
accounts are enabled; their emails are never verified because delivery is deferred. If an
unverified address could align, then registering with a password against *your* Google address
would hand me *your* account. The rule closes it:

- two **verified** addresses that match: one account, aligned automatically
- anything involving an **unverified** address: separate accounts, always
- linking them anyway is a deliberate action taken from account settings **while already signed
  in to the account being linked to**, which needs no email delivery to be safe

So a password account and a Google account with the same address stay separate until someone
signs in to one and links the other. That is the correct amount of friction for a provider whose
addresses nobody has checked.

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
player is signed in to the site (see below; a redirect, not an OAuth dance)
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

The native client cannot rely on a website session. It opens the system browser at the same
`/signin`, with a loopback `return_to`, takes the code, and exchanges it for a long-lived
**device grant** it keeps in the OS keychain — one more row in the broker and no new flow. It
trades that grant for the same kind of ticket whenever it connects. **Both clients present the same thing to the game server**, which
therefore has exactly one code path and no notion of which build it is talking to.

## Not an authorization server

The site and the broker are both ours, deployed by us, on hosts we control. Between two such
services, OAuth2 is ceremony: a token endpoint with client authentication, refresh rotation,
consent, scopes, discovery — machinery whose entire purpose is to let a party you do not trust
act on a user's behalf. Building it to talk to yourself is how a weekend service becomes the
thing you should have bought instead.

So the site signs in through a redirect and one exchange:

```
1.  site    -> broker   GET /signin?return_to=<url>&state=<nonce>
2.  broker              establishes its own session: a provider dance, or a password form
3.  broker  -> site     302 <return_to>?code=<opaque>&state=<nonce>
4.  site    -> broker   POST /exchange { code }        server to server, shared secret
                        -> { account_id, name }
5.  site                sets its own session cookie
```

A **one-time code** rather than a signed assertion in the query string, which is the one place
I would refine "just redirect with a token". The credential in step 3 lands in browser history,
in the site's access log, and in a `Referer` if anything on the landing page is third-party. A
signed assertion there is a bearer token sitting in all three. A code is worthless the moment
it is spent, which is before the address bar has finished rendering. It also means the site
verifies no signatures and fetches no keys: the broker answers in step 4 and that is the whole
of the site's crypto.

The rules that make it safe are short, and all four are load-bearing:

- **`return_to` is matched against an exact allowlist**, not a prefix and not a regex. An open
  redirect on an auth service is how assertions get stolen, and prefix matching is how open
  redirects happen.
- **`state` is generated by the site**, kept in a short-lived cookie, and compared on return.
  Without it an attacker can complete a sign-in *as themselves* in a victim's browser, and the
  victim goes on to use the attacker's account.
- **A code is single use and lives sixty seconds**, the same shape as a game ticket, and is
  bound to the `return_to` it was issued for.
- **`/exchange` is server to server** and authenticated by a shared secret. It is never
  reachable from a browser.

What this gives up is third-party clients. If something we do not deploy ever needs to sign a
player in, this is not enough and the answer is not to grow it — it is to put a real
authorization server behind the same account tables, which this shape does not prevent.

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
- **Whether `password` ships.** It exists so a development environment can make a hundred
  accounts without talking to Google. Everything a public password provider needs on top —
  delivery, reset, captcha, breach lists — is listed above as deferred, and that list *is* the
  decision: `password` goes to production when the list is empty, and not before.

## Where it lives

Its own cargo workspace, beside `web/`, for the reason `web/` has one: a second lockfile is what
makes "deploys on its own schedule" a mechanism rather than an intention. It shares no code with
either product and must not appear in either dependency tree.

```bash
cargo tree -p lc-server | grep -i identity     # must be empty
```

The game server depends on a **public key and a JWT library**, not on the broker.
