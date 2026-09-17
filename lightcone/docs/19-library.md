# The library

Books, to read while you are waiting for something.

A crossing takes real minutes and a build takes real minutes, and both are time the player is
sitting in front of a window with nothing to do. This is what they can do instead of tabbing
away. The books are DRM-free public-domain epubs — Project Gutenberg, mostly — served from the
CDN, catalogued by the shard, and read in a window inside the game.

## Decided: it does nothing

No experience, no bonuses, no unlocks, no crew morale, no skill checks. Nothing in the world
changes because a player finished a book, and no part of the game is gated behind reading one.

That is the design, and it is worth stating first because **almost every hard problem in this
feature disappears with it**:

| problem this would have | why it is gone |
|---|---|
| a client could lie about its progress | there is nothing to win by lying |
| the server must validate a locator | it cannot anyway — it never reads an epub — and does not need to |
| pagination must agree between machines | it does not; each client paginates for its own window |
| reading must be interruptible by the world | it already is; the game does not pause, and the reader does not either |
| a book is game content and needs balancing | it is furniture |

The one thing the feature does owe the player is that it not cost them anything: the flight
readout stays visible, notifications still land, and closing the book puts the keys back.

## Four pieces, in four places

| piece | lives | why there |
|---|---|---|
| the bytes | CDN, `library/<file>.epub` | big, immutable, and wanted by both builds |
| the catalogue | `books.toml`, shipped with `lc-server` | edited by a person, read by everyone |
| progress | `lc_store`, `(account, book)` | per-account, relational, and a checkpoint rather than an event |
| the reader | `lc-books` + `lc-client` | parsing is engine-free; only egui can measure egui's fonts |

---

# The shelf on the CDN

## Not under a build id

[14-hosting.md](14-hosting.md) puts everything under `game/<build-id>/` and never overwrites
it. Books do not belong in that namespace in either direction: under a build id, every wasm
build re-uploads the whole library, and a rollback silently changes which books exist. They are
not part of the client; they outlive it.

```
cdn.<domain>/library/<file>.epub
```

A sibling of `game/`, with the same rule — **nothing is ever overwritten**. That costs nothing
here, because a Gutenberg epub does not legitimately change. A corrected file is a new name.
`Cache-Control: immutable` therefore stays true, which matters because the dev CDN's Caddyfile
applies it to the whole root and a real bucket will too.

One header to add, beside the three the Caddyfile already corrects:

```
@epub path *.epub
header @epub Content-Type "application/epub+zip"
```

CORS already allows the game's origin, and the fetch is the same cross-origin case the wasm and
the sky are.

## Git holds the catalogue, never the bytes

Two scripts, both siblings of `tools/publish-build.sh` and both dull:

- `tools/fetch-books.sh` reads `books.toml`, downloads each `source` into `target/library/`,
  and checks it against the recorded `sha256`. A developer gets the shelf without the repo
  carrying a hundred megabytes of epub forever.
- `tools/publish-books.sh` uploads `target/library/` and refuses to publish over an existing
  name, the way `publish-build.sh` refuses an existing build id.

The hash is the reason both of these are trustworthy: it is what makes the download
reproducible, what catches a Gutenberg re-release changing under a stable URL, and what the
client would check if it ever cached a book on disk.

## Fetching: Bevy already has this

`bevy_asset` ships `WebAssetPlugin` behind the `http` / `https` features. It registers `https`
as an asset source and resolves it through `fetch` on wasm and `ureq` on native, so

```rust
asset_server.load::<Epub>("https://cdn.example/library/frankenstein.epub")
```

is one code path for both builds, and the loader is `SkyLoader` with a different decoder. Three
things to know before wiring it:

- **It must be added before `AssetPlugin`**, or it registers nothing and warns about it.
- It logs a loud warning about loading arbitrary URLs, and the warning is correct. The answer
  is that **the client never receives a URL** — it receives a base from its own shard and a
  bare file name from the catalogue, and composes them after rejecting any name containing a
  slash, a backslash or a dot segment. One function, one test.
- The native `web_asset_cache` feature writes to `.web-asset-cache` in the process working
  directory and never invalidates. That is not a cache a shipped desktop client should have.
  If books should survive a flight without a connection, it is our own cache under the same
  `directories` path the device grant uses, keyed by the `sha256` — and that is a later step,
  not this one.

---

# The catalogue is a file

Titles and authors are written by a person and read by everyone. That is the site's `content/`
pattern, not a table: a `books.toml` shipped with the server, parsed at boot, failing the boot
on a duplicate id or a malformed entry — for the reason the site's slug check exists, that
serving whichever one won is worse than not starting.

```toml
[[book]]
id      = "frankenstein"
title   = "Frankenstein; or, The Modern Prometheus"
authors = [{ name = "Mary Wollstonecraft Shelley", sort = "Shelley" }]
year    = 1818
file    = "frankenstein.epub"
sha256  = "…"
source  = "https://www.gutenberg.org/ebooks/84.epub3.images"
```

- **The id is the file stem.** A second identifier is a second thing that can disagree with the
  first, and the id is already in the URL.
- **`sort` exists because "sort by author" is a requested control.** "Mary Wollstonecraft
  Shelley" sorts under M without it, and under S with it. The default is the last
  whitespace-separated token, which is right for most Western names and wrong often enough
  that the field has to be there to override it.
- **`year` is first publication, not this edition.** A player sorting by year wants the book's
  age, not the transcription's. It is optional; some works do not have one.
- `authors` is a list because many books have several, and the shelf groups by each of them.

The catalogue is data the client renders, so it is also the only place a typo shows up. Boot
validation is: ids unique, file names bare, `sha256` well-formed, every `id` distinct from
every other.

---

# Progress

## The locator is the schema, so get it right first

**Do not store a page number.** A page is a function of window size, font size and the build
that laid it out; a stored page moves when the player resizes the window, and there is no
migration that recovers the sentence they were on.

What is stable is **`(spine_index, char_offset)`** — which document in the spine, and how many
characters into its text. It survives a font change, a window change, a rewrite of the layout
code and a move to a phone. It is cheap to compute from a page break. It needs none of EPUB
CFI's XML-path machinery, which buys a precision this feature cannot use.

For a progress readout and for *jump to page*, derive **locations** the way Kindle does: a
fixed 1 024 characters of body text is one location, counted from the book alone. "Location 340
of 4 210" is then stable forever and identical in every build, while "page 4 of 11" is an
honest statement about the current window and is never written down.

## The server stores a number it cannot check

The shard has the catalogue, not the books. It cannot compute a location, cannot validate an
offset, and does not need to: progress is self-reported, and the feature is worth nothing to
cheat at. `location` and `locations` are stored alongside the locator purely so the shelf can
say "34%" without downloading the book first.

```sql
-- 0005_reading.sql
CREATE TABLE IF NOT EXISTS reading (
    account     text NOT NULL,
    book        text NOT NULL,
    -- The locator: which spine document, and how far into its text. Opaque here.
    spine       integer NOT NULL,
    char_offset integer NOT NULL,
    -- For display only, and reported by the client, which is the only party that has the book.
    location    integer NOT NULL,
    locations   integer NOT NULL,
    read_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account, book)
);
```

`char_offset` rather than `offset`, which is reserved. These rows are not events and
`retention` never touches them.

**No account, no sync.** An anonymous connection's ship has a null account in `ships` and the
same is true here: the reader still works, and it forgets when the process does.

## Two messages, and why they skip the gate

`lc-proto`'s whole argument is that nothing reaches a client except through `Cleared`, because
an event delivered early deletes the game. A book is not an event and no observer sees one, so
`Library` and `Reading` go direct — and **that has to be said in the code**, or the next reader
assumes the gate was forgotten rather than reasoned about.

| direction | message | carries |
|---|---|---|
| server → client | `Library` | `library_base`, and the catalogue |
| server → client | `Reading` | this account's locators, on connect |
| client → server | `SetReading` | one book's locator, debounced |

`library_base` comes from the shard rather than the client's own configuration, mirroring
`cdn_base` on the site's `releases` row: it lets the shelf move CDNs without a client release,
and the client already trusts its shard for everything else.

Adding these bumps `PROTOCOL_VERSION` and re-pins `golden`.

The write is debounced — a page turn every few seconds must not be a message every few seconds
— and flushed on closing the book, on sign-out and on disconnect.

---

# `lc-books`

Engine-free: zip, XML, a document model, locators and the pagination algorithm. No bevy, no
egui, testable with no window. `lc-client` renders it.

## What an epub is, minus the parts we do not do

Read: `META-INF/container.xml` for the OPF path, the OPF for metadata, manifest and spine,
`nav.xhtml` or the NCX for the table of contents, and each spine document as XHTML.

Ignored, deliberately: **CSS, entirely**. A whitelist of five properties is a permanent
negotiation with a thousand publishers' stylesheets, and Gutenberg's typography is not worth
it. Also scripts, embedded fonts, fixed-layout, MathML, and SVG used as a whole page. A book
that needs any of those is a book the shelf does not carry.

## The document model

| block | inline |
|---|---|
| paragraph, heading (1–6), blockquote, list item (ordered/unordered, depth), image, rule, page-break hint | emphasis, strong, code, small-caps, superscript, link |

**Every block carries the `char_offset` of its first character**, counted over the spine
document's body text. That single coordinate is what makes the locator, the table of contents
and a future in-book search the same mechanism rather than three.

New dependencies: `zip` with `default-features = false, features = ["deflate"]` — the defaults
drag in bzip2, zstd, lzma and AES, none of which an epub uses — and one XML parser, where both
`quick-xml` and `roxmltree` are already in the tree transitively. Everything else is already
here. All of it is pure Rust and compiles for wasm, which is a claim to verify with the first
build rather than to trust.

## Pagination is a function, not a widget

The measurer is a trait, because only the client can measure the client's fonts:

```rust
pub trait Measure {
    /// The laid-out rows of one block at this width: each row's height, and the char offset
    /// it begins at.
    fn rows(&self, block: &Block, width: f32) -> Vec<Row>;
}
```

**Rows, not blocks.** A paragraph routinely exceeds a whole page, so the unit a page break
lands on is a line, not a paragraph — and a row that knows its own char offset means the break
*is* the locator, with no second derivation to disagree with the first.

Three rules the algorithm follows:

- **Never paginate the book, or even the chapter.** Break forward from the current anchor until
  the page is full, and stop. The list of breaks already walked is a cache that makes *previous
  page* a lookup; it is thrown away whole on a resize or a font change, and the anchor is a
  char offset, so the player stays on the same sentence.
- **Widows and orphans**: a heading does not end a page, and a single row of a paragraph does
  not begin or end one where moving it costs less than a third of a page.
- **An image never splits.** One taller than the column is scaled to fit the page; one that
  does not fit beside the text starts a new page.

This is where the bugs will be, and it is testable headless with a measurer where every row is
ten units tall — the same trick as the sky chunk's equivalence test, and the reason the
algorithm is in a crate with no window in it.

---

# The reader window

## Rendering

One `LayoutJob` per block, drawn into a column that is centred in the panel and capped at a
comfortable measure — around 66 characters, which is a width in `em`, not in pixels. No
justification: egui has none, and faked justification without hyphenation is worse than a ragged
edge.

Images live inside the zip, so they do not go through the asset server. `lc-books` hands out
the bytes and the media type; the client decodes and uploads a texture, behind an LRU cap,
because an illustrated edition can carry a few hundred.

## The client has no fonts, and this changes that

[14-hosting.md](14-hosting.md) records it as a fact about the download: no textures, no skybox,
no fonts, so a first play is 6.4 MB and 94% of it is the binary. egui's built-in face is a UI
font — correct for a readout, wrong for forty minutes of prose.

So the reader ships one serif, OFL-licensed, subset to Latin-1 and the punctuation Gutenberg
actually uses: roughly 200 KB, and its licence file ships beside it. **It is loaded as an asset
when the reader is first opened, not embedded in the binary**, so a player who never opens a
book never pays for it and the first-play figure above is unchanged. Installing a font into
egui at runtime is one call against `FontDefinitions`; doing it lazily is what keeps this from
being a 3% tax on every player.

## Controls

Two panels. `Panel::Library` is the shelf and `Panel::Reader` is the book; the shelf is where
sorting lives, and opening a book is the only thing it does.

| shelf | |
|---|---|
| sort | title, author, year, recently read |
| filter | a text field over title and author |
| per book | title, authors, year, a progress badge, and whether it is downloaded |

| reader | key |
|---|---|
| next page | `Space`, `→`, `PageDown` |
| previous page | `Backspace`, `←`, `PageUp` |
| chapter list | `C`, and it is a list of the TOC with the current entry marked |
| jump to location | a number field, against the stable location count |
| close | `Esc` |

**The binding table becomes mode-dependent, and that is a real change.** Arrow keys turn the
view today, and `input.rs` is deliberately the only place that knows about keys. So `Reading`
is a mode that selects a second table, entered when the reader takes focus and left when it
closes. egui's `EguiWantsInput` cannot do this on its own: it reports keyboard interest only
when a *text field* has focus, and a page of prose has none.

Everything the surface does is an `Action` — `OpenBook`, `CloseBook`, `TurnPage(i32)`,
`JumpToChapter(usize)`, `JumpToLocation(u32)`, `SetShelfSort(Sort)`. The filter text and the
scroll position of the shelf are the surface's own, the way the password form owns what is
being typed.

## Style, and one deliberate departure

[18-ui-style.md](18-ui-style.md) puts a panel at 92% opacity so the world shows through. **A
reading surface is opaque.** Prose over a drifting starfield is unreadable in a way a readout
over one is not, and the rule's own reason — that a panel should feel like part of the scene —
argues the other way for a page of text.

What does not change: the flight readout and the staleness figure stay visible, and
notifications still land on top. The entire premise of the feature is that the player is
waiting for something, so the thing they are waiting for must be able to interrupt them.

And it must be photographable, because a text layout nobody photographed is a text layout
nobody has checked:

```bash
cargo run -p lc-client --bin lightcone -- --panel reader --book frankenstein \
    --shot /tmp/reader.png --frames 90
```

---

# Order of work

Each step is useful on its own, and the fun one does not wait for the server.

| # | step | done when |
|---|---|---|
| 1 | `lc-books`: zip, OPF, spine, TOC, the block model, locations, the paginator over `Measure` | a headless test paginates a real Gutenberg epub and round-trips a locator |
| 2 | the shelf on the CDN: `books.toml`, the two scripts, the Caddyfile header | `curl` returns an epub with the right type and an immutable cache header |
| 3 | the reader window: `WebAssetPlugin`, `EpubLoader`, the serif, the panels, the mode | `--panel reader --book <id> --shot` is a page of prose |
| 4 | the catalogue and progress over the wire: two messages, `0005_reading.sql`, the debounce | signing in on a second machine opens to the same sentence |
| 5 | the shelf's sorts, the TOC, jump to location, the progress badges | the controls above all exist |

---

# Open

- **In-book search.** The block model already has the coordinate for it; the UI and the
  incremental scan do not exist. Cheap later, and not step 5.
- **Bookmarks and highlights.** A second table with the same locator shape. No reason to build
  it before someone asks.
- **Downloading for offline.** A real cache under the config directory, keyed by `sha256`. The
  browser gets this free from HTTP; the desktop does not.
- **Whether the shelf is ever social** — what other players are reading. It would be charming
  and it is a privacy decision, not a technical one, so it stays shut until it is made
  deliberately.
- **Whether a book is ever diegetic**: a station library you have to be docked at to browse.
  Attractive, and it contradicts "no game reason" the moment it gates anything. Nothing in this
  design forecloses it; the catalogue would grow a condition and the reader would not change.

## Not in this

Uploading your own epubs. It is the obvious next ask and it is a different feature: arbitrary
user files reaching an asset loader is the exact thing `WebAssetPlugin` warns about, and it
would need a size cap, a sanitiser and somewhere to put the bytes that is not the shared CDN.

## A note on Project Gutenberg

Their epubs carry a licence header and footer inside the text. Leaving them there costs a
screen and requires nothing of anyone; **stripping them is what triggers the clause about
removing every reference to Project Gutenberg**, which is not a trade worth making. So the
files are published byte-for-byte as downloaded — which the `sha256` in the catalogue also
happens to prove — and the table of contents lands the player on Chapter 1.
