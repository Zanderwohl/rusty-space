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

Three rules that make the list honest:

- **A provider in the list must be configured, or the process refuses to start.** Google needs a
  client id and secret; `password` needs nothing, which is exactly why it needs to be named
  explicitly rather than inferred from configuration being present.
- **A provider in the list must be *implemented*, or the process refuses to start.** Credentials
  are not the only thing a provider needs. `discord` has a place in the enum and no dance
  written, and a deployment that names it should fail at boot rather than render a button that
  cannot work.
- **`password` is off unless listed.** Off by default means shipping without having thought
  about it leaves it off, which is the right direction for the failure to point.

An upstream provider also needs `LC_IDENTITY_PUBLIC_URL`, because the redirect URI it registers
with the provider has to be built from somewhere. Not from the request: `Host` is whatever the
client sent, and an authorization code is delivered to whatever that builds.

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

## The upstream dance

Authorization code with PKCE, and nothing else. The broker is a confidential client — it holds a
secret and runs on a host we control — so PKCE is not load-bearing here the way it is for a
public client. It is included because OAuth 2.1 requires it of everyone, it costs a column and
fifteen lines, and it closes authorization-code injection, which a fixed redirect URI does not.

A dance in flight is a **row**, not a cookie and not process memory. A cookie would be a second
thing to get `SameSite` right about across a cross-site redirect; process memory would mean a
dance only completes if it lands on the replica that started it. The row holds the verifier and
where the sign-in was headed, keyed by the **digest** of the `state` — knowing a pending state is
enough to complete somebody else's dance, so it is stored like every other credential here.

`return_to` is checked when the dance starts **and again** when it finishes. Ten minutes pass in
between, the allowlist can change inside them, and the second redirect is the one that actually
carries a credential.

### The ID token's signature is not checked

This looks wrong and is not. The token arrives in the body of a response to a request *the broker
made*, over a TLS connection it opened to the provider's token endpoint, authenticated with its
client secret. There is no path by which anyone else could have put a token there, so a signature
proves nothing the transport has not already proven. [OIDC Core §3.1.3.7] rule 6 says exactly
this, for exactly this case.

What it buys is not laziness. Verifying would mean a JWKS fetch, a key cache, a rotation story,
and a new outage mode where nobody can sign in because a key server is unreachable — and it would
put this code within reach of **algorithm confusion**, which is *the* JWT bug and one this
repository has already been bitten by once.

The claims are still checked, because "it came from the right socket" is not "it says what it
should": `iss` against the provider's configured issuers, `aud` against our own client id, `exp`
against the clock. Each of those is a real attack if it is missing, and each has a test that
names the error it expects rather than asserting merely that something failed.

[OIDC Core §3.1.3.7]: https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation

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
- **claims** `sub` (account id), `name`, `iat`, `exp`, `aud`, `jti`, `perm`. Nothing else. A
  claim the game does not need is a claim that leaks.
- **`perm`** is the account's permission level, from `accounts.permission`: 0 a player, 1 an
  admin. An integer because it will grow into levels or groups. The broker only carries it; a
  game server decides what a level allows, and today a shard lets an admin issue development
  actions. There is no interface for it: the first admin was set by migration `0003`.

The native client cannot rely on a website session. It opens the **system browser** at the same
`/signin`, with a loopback `return_to`, takes the code, and exchanges it for a long-lived
**device grant** it keeps in the OS keychain — one more row in the broker and no new flow. It
trades that grant for the same kind of ticket whenever it connects, so the sign-in happens once
rather than per launch.

The system browser, and not a window inside the game, because that is the only way an upstream
provider can work at all: Google will not authenticate into an embedded view it cannot show its
own address bar in, and should not.

**The password provider is the exception, and it is an argument against itself.** A local
password form can go directly in the game's own modal — there is no third party to redirect to,
and for the case the provider exists for, making a hundred development accounts, a browser round
trip per account is the whole cost. But it teaches a player to type a credential into a game
window, which is exactly the reflex phishing relies on. That is a second and independent reason
to keep `password` out of production, alongside the deferred list below: not merely that it is
unfinished, but that the shape it needs in the native client is a shape a public deployment
should not train anyone into. **Both clients present the same thing to the game server**, which
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

## The pages look like the site, without sharing a line with it

A sign-in page that does not look like where it came from is the exact thing a person is told to
be suspicious of, so these pages carry the site's palette, its typefaces and its wordmark. They
cannot carry its stylesheet: two workspaces, two docker build contexts, and a second copy of
`web/`'s source tree in this image would be the coupling the separation exists to avoid.

So `_tokens.scss` is **copied**, and a test compares the two files byte for byte
(`assets::tests::the_tokens_are_the_sites_tokens`). A copy with nothing watching it drifts; that
test is what watches it. Anything genuinely broker-only — there is one thing, a colour for a
refusal, which the site has never needed because nothing there fails — goes in `_status.scss`
instead, so the copied file stays a copy.

The sheet is compiled by `build.rs` rather than at boot, which is the one place this differs from
the site on purpose:

- a SCSS error is a failed **build**, not a failed deploy;
- the image ships no `static/` tree, so nothing at runtime reads a file;
- the cache-busting URL segment is a digest of the CSS itself rather than a git revision, which
  is what makes `Cache-Control: immutable` unconditionally true. The site's build id does not
  change when a stylesheet does, which is why it only sends that header in production.

Three smaller decisions, each of which is a thing not to undo:

- **The wordmark is text, not a link.** Every other destination a browser can reach from these
  pages is on the `return_to` allowlist. A masthead would be one that is not.
- **`referrer: no-referrer` and `robots: noindex`.** Every URL here carries a `return_to` and a
  `state`; a `Referer` hands both to whatever is linked, and an indexed sign-in page is one
  reached without the query that makes it work.
- **No JavaScript**, the same rule the site keeps and for a stronger reason: a page that collects
  a password should be a document.

## What this does not do yet

Named so they are decisions rather than omissions:

- **Entitlements.** Whether an account may play at all, or on which shard, is a game question
  and belongs in the game's database keyed by account id. The broker answers who, not what.
- **Multiple characters.** [09-open-questions.md](09-open-questions.md) has not settled whether
  a player ever inhabits more than one ship. `Welcome` returning a single `ship_id` assumes not;
  if that changes it becomes a list and the client picks, which is a protocol change and a
  version bump.
- **Reaping expired rows.** `signin_codes` and `upstream_flows` are deleted when spent and
  filtered by expiry when read, so a stale row is never honoured — but an abandoned one is never
  swept either. Both tables carry an expiry index for the job; nothing runs it yet.
- **A broker session.** Each `/signin` is a fresh one, so a player who signs in to the site and
  then launches the desktop client authenticates twice. The upstream providers paper over this
  with their own sessions; `password` does not.
- **Account deletion.** Deleting the broker row orphans the game's rows rather than cascading,
  because a cascade across three databases is the coupling this design spent its whole budget
  avoiding. The game reaps orphans on its own schedule.
- **Whether `password` ships.** It exists so a development environment can make a hundred
  accounts without talking to Google. Two conditions, not one. The deferred list above —
  delivery, reset, captcha, breach lists — must be empty. And the native client's in-modal form
  must go, because a public build that asks for a password in a game window trains the reflex
  that phishing exploits. The first is work; the second is a decision about what the game
  teaches, and it is the harder of the two.

## Levels, bans, and the console that works them

This section reverses a decision the "what this does not do yet" list above used to hold.
**Bans were going to be the game's**, on the reasoning that a ban is the game refusing a valid
identity and the broker should not learn what a player did in a world it knows nothing about.
That reasoning is sound about *entitlement* and wrong about what was actually wanted. What is
wanted is that a banned account **cannot sign in** — not to a shard, and not to the website
either — and is told how long that lasts. Nothing but the broker can refuse a sign-in. So the
ban lives here, and the layering is preserved in a different place: the broker stores a ban's
**reason** as one of a closed set of eight names, and never learns what happened in the world
beyond that.

### The level is a ladder, and lower is higher

`accounts.permission` was 0 or 1. It is now 0 to 3, with a check constraint:

| | | |
|---|---|---|
| 0 | Player | administers nothing |
| 1 | Owner | the most senior |
| 2 | Administrator | |
| 3 | Moderator | trusted with people, not with the sky |

**The integers run opposite to seniority**, which is the one thing about this column that is
easy to get wrong: `permission >= 1` reads as "is an administrator" and is right, while
`permission > other` reads as "outranks" and is exactly backwards. Nothing above the schema
compares them directly. `lc_identity::level::Level` carries the ordering and offers
`outranks` as its only comparison — deliberately no `Ord`, because a derived one would make
`a > b` a compiling, plausible, wrong authorisation check.

`lc_server::ability::Level` is the same type again, on the far side of the workspace boundary.
It is duplicated for the reason the ticket's claim set is duplicated: the game server must not
depend on the broker. The two agree by this document and by a test on each side.

### Who may do what

Three rules, all of them in `lc_identity::ability` as pure functions of two levels, and all of
them asserted exhaustively rather than argued about:

- an administrator may **promote** anyone up to their own level, so a 2 hands out 2 and 3 and
  never 1;
- an administrator may **demote** anyone strictly below their level, so a 2 may demote a 3 and
  may not touch another 2;
- an administrator **cannot be banned**. De-admin first, which is a separate act by somebody
  senior and leaves a line in the log.

Two consequences fall out, and both are intended. **Nobody changes their own level**, because
demotion needs an actor who outranks the subject and nobody outranks themselves — so a stray
click cannot strand a shard with no owner. And **an owner cannot be demoted by anyone**,
including another owner, because there is nobody above 1: removing an owner is a row changed by
hand against the database, deliberately, so that the one irreversible administrative act is not
reachable from a web page.

### A ban is a row, and they are served concurrently

An account can hold several at once, each on its own clock. Three weeks for one thing and two
months for another are two facts, both true, and a single `banned_until` column would lose the
reason the longer one was issued.

What a refused sign-in is told is the **furthest** expiry among the bans in force — the nearest
would be a lie the person discovers three hours later — together with how many there are, so
somebody who waits one out and is refused again does not conclude the service is broken. What
they are **not** told is the reason: a ban's public reason belongs in whatever is said to them
out of band, and its private case notes sit one column away from it.

The check runs at every point that turns an account id into something usable:
`lc_identity::signin::admitted`. Both password forms, the end of an upstream dance, the site's
code exchange, the device-grant exchange, and **ticket minting**. The last matters most and is
the one a sign-in-only check would miss: a site session is a fortnight and a device grant is
ninety days, so without it a ban issued to somebody already signed in would not reach them
until after it had expired.

A store failure is not a refusal. A database that cannot be reached must not lock every account
out of the game.

### The console is a second service

`auth/lc-admin`, in the broker's cargo workspace, sharing its database and its lockfile and
deploying as its own container on its own hostname. Not `/admin` on the broker, because the
broker is the service every player reaches to sign in and an administration console should not
share an origin, a process or a dependency tree with it.

**`lc-identity` owns the schema.** It holds every migration and applies them at its own boot;
the console reads and writes tables it did not create and runs no migrations. Two services
migrating one database is two advisory locks and a race between whichever container starts
first.

The console is an ordinary first-party client of the broker, exactly as the website is: it
sends a browser to `/signin`, gets a code, and spends it for an account id. There is no separate
administrator login. What makes somebody an administrator is a column — which is why the level
is read from the database on **every request** rather than carried in the session cookie. A
level in the cookie would mean a demotion took effect when the demoted person next signed in,
which is to say at a time of their choosing.

**A player who signs in is never given a session.** The level is read before anything is
sealed, so somebody who is not an administrator is told so and left holding nothing — rather
than holding a cookie whose only use is to be rejected by every page. The other door is an
administrator demoted mid-session: the extractor refuses them *and clears the session*, because
reading the level per request exists to make a demotion take effect now, and that has to
include ending the session it just invalidated.

Both refusals name a way out. The console's only navigation is a masthead that renders for
administrators, so a refusal without a link is a dead end — no menu, and nothing on screen
naming the address that would get you off it. `/signout` existed from the first commit and was
linked from exactly one place, which was the page a refused visitor never sees.

Its session is eight hours, not the site's fortnight: the revocation window of a signed cookie
is its lifetime, and an administrative session is a working day at a desk.

### The pages

Server-rendered HTML, SCSS, and htmx 4. One TypeScript module and one small component, compiled
by a stage of the container build; nothing renders in the browser.

**The URL is the state.** Every filter, the sort and the page number live in the query string
and nowhere else, so a link pasted into a chat window reproduces what the sender was looking at.
`lc_admin::listing::Listing` is the one definition of what that query string means — the page
handler, the partial handler and the browser all read it, the last through a `data-listing`
attribute carrying the same defaults the canonical form drops parameters against.

Paging is **`limit`/`offset` with a `count(*) over ()` window**, one statement for the rows and
the total. Every ordering ends in a tie-break on `accounts.id`, without which two accounts
created in the same second can swap places between the query for page 1 and the query for page
2 — which shows one twice and the other never.

Everything works with scripting off: every heading and pager link is a real `href`, the filter
form is a real `method="get"` form, and every act is a real `method="post"` form that ends in a
redirect. htmx makes those same URLs into swaps, and the TypeScript keeps the address bar in
step with them, which is the one thing neither the server nor htmx can do.

### htmx is in the image, not on the CDN

Vendored: 36 KB of `htmx.min.js` committed under `lc-admin/static/vendor`, served by the
console itself at `/v/<digest>/scripts/htmx.js` with a one-year `immutable` header. No page
here references an origin this project does not run.

Not on `cdn.<domain>`, and that was asked about rather than assumed. The CDN is for **game
builds** — `cdn.<domain>/game/<build-id>/…`, never overwritten, promoted by a row in the site's
database ([14-hosting.md](14-hosting.md)). htmx is not versioned on that axis and is not
promoted, and putting it there would cost the property that makes the current arrangement worth
having: htmx ships in the same image as the binary that needs it, behind a **single digest
computed over the stylesheet, the script and htmx together**. Deploying the console deploys the
exact htmx it was built against, and rolling back rolls back all three. Served from the CDN it
would be a second container, a second volume and a second publish step, with a new state in
which the console is up and its script is a 404 — and a cross-origin fetch on the one surface
most worth keeping same-origin.

**The committed copy is checked against `package-lock.json`.** `npm run build` refuses when the
two differ, the container's ui stage runs the same check, and the runtime stage then takes htmx
out of `node_modules` rather than out of the repository — so "the image ships what the lockfile
pins" holds by construction. To take a new htmx: bump the dependency, `npm run vendor`, commit
both. This exists because the alternative is the `_tokens.scss` situation, where a copy with
nothing watching it drifts and nobody finds out until it matters.

The argument would change if a second service wanted htmx. Three copies of it is the same
problem again, and that is the point at which a shared origin starts paying for itself.

### On the game side

`lc_server::ability` is the same idea over the game's vocabulary: one table, read from one
function, saying whether a connection may command a craft, speak as one, grant it energy or
stage a scene. `Act::of` maps every `Order` variant to one of those with an exhaustive match
and no catch-all arm, so a new order is a compile error rather than an ungated action.

Two rules there are worth stating here:

- **Commanding a craft is ownership at every level.** An administrator may not fly somebody
  else's ship. Every act a craft performs becomes an event attributed to that craft, and
  nothing downstream carries who at a keyboard caused it — so an administrator flying a
  player's ship would put a burn in the record that the record says the player made. A
  moderation tool that falsifies the evidence is not a moderation tool. The remedy for a player
  who should not be flying is the ban above, which leaves a row, a reason and a name.
- **Development actions are ranked.** Granting energy and staging a scene need level 2 or
  better, which a moderator does not hold.

## Where it lives

Its own cargo workspace, beside `web/`, for the reason `web/` has one: a second lockfile is what
makes "deploys on its own schedule" a mechanism rather than an intention. It shares no code with
either product and must not appear in either dependency tree.

```bash
cargo tree -p lc-server | grep -i identity     # must be empty
```

The game server depends on a **public key and a JWT library**, not on the broker.

The administration console is the one thing that *does* depend on `lc-identity`, as a path
dependency inside the same workspace. That is deliberate and is the opposite trade from the one
above: the two write the same tables, so a second copy of `Level`, of a ban's reasons or of the
promotion rules would be a copy with nothing watching it. Where a workspace boundary forces a
copy — the ticket claims, and `lc_server::ability::Level` — this document and a test on each
side are what hold them together. Where there is no boundary, a dependency is cheaper and
truer.
