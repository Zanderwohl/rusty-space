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

## The books are in the repository

`crates/lc-client/assets/books/`, bytes and all — six of them, 43 MB, most of it two Twain
editions carrying a hundred and seventy-eight scanned plates each.

**The alternative was tried on paper and was worse.** The plan here used to say git holds the
catalogue and never the bytes, with a `fetch-books.sh` that re-downloaded each book from its
`source` and checked a `sha256`. It does not survive contact with the files: these six came
through Apple Books, three of them arrived unpacked and were re-zipped locally, and a fresh
Gutenberg download is a different arrangement of bytes with a different hash. The script would
have fetched *a* copy of each book and failed the check on every one. A game's resources are
heavy; a repository is where they live.

So there is one script, a sibling of `tools/publish-build.sh` and just as dull:
`tools/publish-books.sh` uploads the directory and refuses to publish over an existing name, the
way `publish-build.sh` refuses an existing build id.

Two things this does **not** change:

- **A book is still not a build asset.** `tools/build-wasm.sh` stages named directories —
  the shaders, and a sky it generates — rather than copying the asset root, so the books do not
  land under every build id on the CDN. If that ever becomes a `cp -R`, this is what breaks.
- **`sha256` is still worth carrying**, and now it can be right: it is recorded from the file in
  the repository, which is the file every player reads. `source` stays beside it as provenance
  for a person, not as an input to a tool.

The one thing in that directory that is *not* committed is the reading face, which is not ours
to redistribute.

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
authors = [{ name = "Mary Wollstonecraft Shelley", sort = "Shelley, Mary Wollstonecraft" }]
year    = 1818
file    = "frankenstein.epub"
sha256  = "…"
source  = "https://www.gutenberg.org/ebooks/84.epub3.images"
```

- **The id is not the file name.** The first real shelf contains `pg2488-images-3.epub`, and
  two files whose names differ from their titles by a lost colon. The id is written down, seeded
  from the title; `file` is a separate field and the two are allowed to look nothing alike.
- **`sort` exists because "sort by author" is a requested control.** "Mary Wollstonecraft
  Shelley" sorts under M without it, and under S with it. **The books already carry it** — as
  `opf:file-as` in EPUB 2 and a `<meta property="file-as" refines>` in EPUB 3 — so the importer
  seeds this field rather than a person typing it. The fallback when a book is silent is the
  last whitespace-separated word, which is right for most Western names and wrong often enough
  that the field has to exist.
- **`year` is first publication, not this edition, and no book on the shelf knows it.** All six
  of the first files state a `dc:date` between 1993 and 2008: the day Gutenberg posted the
  transcription of a book written in the 1870s or 1880s. The importer emits it as a comment to
  be corrected, never as the answer. It is optional; some works do not have one.
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
| server → client | `Library` | the shelf's base, and the catalogue |
| server → client | `Reading` | this account's locators, on connect, **most recently read first** |
| client → server | `SetReading` | one book's locator, on every turn |

That ordering is load-bearing: it is the only record of recency on the wire, and it is what lets
the shelf offer "recently read" without either end having to agree about whose clock a timestamp
would be in. All three are appended variants — the discriminants above them are what the goldens
are pinned at, and a version bump is not a licence to renumber them.

`library_base` comes from the shard rather than the client's own configuration, mirroring
`cdn_base` on the site's `releases` row: it lets the shelf move CDNs without a client release,
and the client already trusts its shard for everything else.

Adding these bumps `PROTOCOL_VERSION` and re-pins `golden`.

**A page turn is written down the moment it happens.** Not on an interval: a page read and then
lost to a crash is a page read twice, and the whole point of a bookmark is the case where nobody
got to close anything cleanly. Putting a book down reports at once too, and so does the first
sight of one — opening a book at chapter nine and dropping the connection four seconds later
should not record chapter one.

What does **not** report, and does not even move the bookmark, is a reflow. A narrower window
puts the same sentence on a page that starts a few characters earlier, and writing that down
would walk the mark backwards every time the window was dragged. Resizing is not reading.

An interval survives as a backstop against something else moving the offset without asking:
every inbound message is charged against a connection's budget — two a second sustained, thirty
in a burst — so a change arriving every frame would spend the lot and then be throttled, which
is how a bookmark gets lost by trying too hard to save it.

The rule is a value rather than a system — `Reporter::tick` — because a system holding a socket
cannot be tested and the rule is the part worth being sure of. Writing the tests found two faults
in it: the first bookmark for a book waited out the interval, and opening a second book overwrote
the first one's unsent place.

Past the client, a bookmark has the same exposure as everything else: the shard holds it in
memory and writes it at the next checkpoint, about twenty seconds out. A ship's position is no
safer, and making a book safer than the ship it is being read on would be a strange place to
spend a write.

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

**A chapter is not a spine document**, and assuming it is breaks the contents menu on real
books. *The Gilded Age* puts seventy-three chapters in nine documents and distinguishes them by
nothing but the fragment on the href. So the parse also records every element id against the
offset it sits at, and a contents entry resolves to a full locator rather than to the head of
whatever document happens to contain it.

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

Four rules the algorithm follows:

- **Never paginate the book, or even the chapter.** Break forward from the current anchor until
  the page is full, and stop. A resize or a font change throws the page away and lays out the
  anchor again, and because the anchor is a character offset the player stays on the same
  sentence.
- **Every page holds at least one row, whatever the frame.** This is not typography, it is
  termination: a page that can be empty is a page that does not advance, and pagination that
  does not advance never ends. A window too short for a single line still pages, one line at a
  time.
- **Widows and orphans**: a heading does not end a page, and a single row of a paragraph does
  not begin or end one — refused when it would cost more than a third of a page, which is the
  difference between typesetting and a hole.
- **An image never splits.** One taller than the column is scaled to fit the page; one that
  does not fit beside the text starts a new page.

This is where the bugs will be, and it is testable headless with a measurer where every row is
one unit tall — the same trick as the sky chunk's equivalence test, and the reason the
algorithm is in a crate with no window in it. `lc_books::grid` is that measurer, and it is not
only for tests: it prints a page to a terminal, and a page that can be printed is a page that
can be diffed.

### Backwards is laid out backwards

*Previous page* looks like a cache problem and is not one. The obvious implementation — lay
pages out forward from a guessed origin and keep the last one that ends in time — **is wrong,
and a test written against a synthetic chapter caught it**: a tiling depends on where it
started, so the page it produces need not end where the reader actually is, and the rows in
between belong to no page at all. Going backwards from the reader's own position, a row at a
time until the frame is full, cannot do that; the page it returns ends where they are by
construction.

The page it returns is not the page the forward tiling would have produced, and that is
allowed. What is guaranteed is what a reader can tell: nothing falls between the two pages, and
turning forward again returns to the page they came from. Its foot is a break that was already
tidied on the way forward and must not move again; its head is still free, so that is the end
the widow rule is applied to.

### A cursor is not a locator

Traversal is by `(block, row)` and what gets saved is a character offset, and they are not the
same thing — **a plate and the paragraph beneath it begin at the same character**, because a
plate is made of no characters at all. Paging forward has to tell them apart or it repeats one
forever; a bookmark cannot, and does not need to. Where the ambiguity is real, reopening
resolves to the earlier block: being shown a plate twice is a smaller wrong than never being
shown it.

This is the one place the two coordinates touch, and finding it took six real books rather than
a fixture — which is the argument for checking a paginator against Twain, who was not writing
test data.

**And it bit again, in the client, for the same reason.** The reader kept its live position as a
character offset alone, so resolving it always landed on the first block sharing that offset —
which meant a page after a plate resolved back to the plate. Huckleberry Finn opens with two
plates in a row, both at character 76, and so does the paragraph after them: the plate drew for
a frame, the position resolved backwards, and the book could not be read past it. Turning back
and jumping to a chapter both worked, because neither goes through the offset.

So the client carries the block index alongside the offset while a chapter is open. The index is
a fact about the parsed document and survives a resize, where a row index would not; it is
dropped on a jump or a fresh open, when the offset really is all there is. A saved bookmark is
still just the offset, and still resolves to the first block that shares it — being shown a
plate again on reopening is the smaller wrong, and the one that cannot trap anybody.

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

So the reader ships **two** faces, both OFL: **Faustina** for the page and **Libre Baskerville**
for titles and headings. A title set in the face the body is set in is a title that does not look
like one, and Baskerville is wider and heavier in exactly the way a line you look at wants and a
page you read does not.

Five files, 416 KB, asked for together when a book is first opened — a page half in one face and
half in another, for the second between them arriving, would reflow under the reader. Each is
optional and falls back a step: no italic means the body sheared, no body at all means the
interface font, so a build without fonts is plainer and never broken.

**Static cuts, not the variable files.** Both families ship a `VariableFont_wght` and neither is
usable here: `epaint::FontData` carries a file, a face index and a scale tweak, and nothing in
egui calls `ab_glyph`'s `set_variation`. A variable font renders at its default instance and
costs half again as many bytes to do it. What the static cuts buy is **real italic and real
bold** — the alternative, and what this had before them, is egui shearing the regular.

Two numbers that go with them: the body is set at 18 points, and **the column is capped at
thirty-four times that**, centred in whatever the window gives. A line of prose stops being
readable somewhere past seventy characters, and a maximised window would otherwise set a book at
a hundred and forty. **It is loaded as an asset
when the reader is first opened, not embedded in the binary**, so a player who never opens a
book never pays for it and the first-play figure above is unchanged. Installing a font into
egui at runtime is one call against `FontDefinitions`; doing it lazily is what keeps this from
being a 3% tax on every player.

## Controls

**One window.** It shows the shelf, or it shows a book off it — the same case, the same page,
the same two bezels. A shelf and a book are one object a player picks up, and two windows for it
was two things to arrange on a screen and two things to close. It opens at 500 by 800, the shape
of a book rather than of a window, and the player can widen it.

Closing walks that hierarchy: out of a book is back to the shelf, out of the shelf is the device
put away. `Escape` and the shelf button are the same step.

**A field being typed into owns the keyboard, all of it.** The shelf's filter is the only one in
this client, and without that rule, typing the name of a book closes the window on `b` and turns
a page on the space bar.

| shelf | |
|---|---|
| sort | title, author, year — and recently read, once the server is keeping progress |
| filter | one line, over title, author, filing name, subject and year |
| per book | title, by-line, year, and whether it is the one open |

**Every word has to land somewhere, in any order and in any case, as a partial match.** So
`twain miss` finds the Mississippi novels and `verne sea` finds the one about the sea, which is
how anyone actually looks for a book they have half-remembered. The subjects come from the
transcription's own `dc:subject` headings; they are searched and never displayed, because
`London (England) -- Fiction` is a library heading and not a genre.

**A title files under its first real word.** `The Gilded Age` belongs under G. Every library in
the world does this, and a shelf that does not has a third of its stock under T.

| reader | key |
|---|---|
| next page | `Space`, `→`, `PageDown` |
| previous page | `Backspace`, `←`, `PageUp` |
| chapter list | `C`, and it is a list of the TOC with the current entry marked |
| jump to location | a number field, against the stable location count |
| close | `Esc` |

**The reader lays a few bindings over the cockpit's, and claims nothing else.** Arrow keys turn
the view, so a book has to take them — but the first cut of this swapped the whole table, which
took the telescope, the system window and the time controls with it. Worse, every panel added
afterwards would have had to be remembered in a second place to keep working. So the reading
table is an *overlay*: the paging keys, `C` for the contents, and `Escape` and `B` for the way
out. Anything it does not name falls through, and a test asserts exactly that — no key outside
the overlay changes meaning while a book is open.

With the shelf showing rather than a book, only the way out is claimed: there are no pages to
turn, so the arrows stay with the view.

egui's `EguiWantsInput` cannot decide any of this, but it does decide one thing: while the
filter field has focus the keyboard is entirely its own, or typing the name of a book closes the
window on `b` and turns a page on the space bar.

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
| 1 | **built.** `lc-books`: zip, OPF, spine, TOC, the block model, locations, the paginator over `Measure` | a headless test paginates a real Gutenberg epub and round-trips a locator |
| 2 | **built.** the shelf on the CDN: `publish-books.sh`, the Caddyfile header, the client's HTTP asset source | `curl` returns an epub with the right type and an immutable cache header |
| 3 | **built, less the CDN.** the reader window: `EpubLoader`, plates, the serif, the panel, the mode | `--book <id> --shot` is a page of prose |
| 4 | **built.** the catalogue and progress over the wire: three messages, `0005_reading.sql`, the reporting rule | signing in on a second machine opens to the same sentence |
| 5 | **built, less the badges.** the shelf's sorts and filter, the TOC, jump to location | the controls above all exist |

Step 4 is `lc_server::library`, `lc_store::reading` and the two client systems that take the
shelf and report a place. Three things it taught:

- **A location is a fact about the book, not the chapter.** Saying where someone is means
  knowing how long everything before them is, and that means parsing every chapter. Doing it on
  opening costs a few hundred milliseconds on a long book — a visible hitch — so the client
  measures **one chapter a frame** and reports nothing until it has finished, which takes under a
  second and shows as nothing at all.
- **The shelf keeps its own copy of what it just sent.** The shard states bookmarks once, on
  connecting, so a shelf that waited to be told would show yesterday's place for the book being
  read right now.
- **A developer's database is one database and their branches are many.** The migration test
  asserted that the number of recorded steps matched the number this build ships, which fails
  the moment another branch has touched the same local Postgres — as one has, with three steps
  this branch has never heard of. It now asserts that *this build's* steps are all applied,
  which is the thing that was meant.

Step 3 is `crate::library` and `crate::reader` in the client. Three things it taught:

- **A page turn cannot be an action.** Turning one means laying it out, and only the surface
  with the fonts in it can do that — so `TurnPage` records a request and the reader spends it on
  the next frame. A jump to a chapter is spent somewhere else again, beside the fetch, because
  what it changes is which chapter is loaded: spent next to a page turn, the turn runs off the
  end of the chapter being left. That was a real bug and the photograph is what found it.
- **Measure the rows the way the painter draws them.** A row's `pos.y` is rounded to the pixel
  grid and its `size.y` is not, and a paginator fed one while the painter uses the other shows a
  sliver of the next line at the foot of every page.
- **Size the window before drawing into it.** Content sized from what is left inside a window
  that grows to fit its content is a loop whose fixed point is a window taller than the screen.

The catalogue arrives the same way a book does — an asset, parsed by a loader, held in a
resource — so step 4 replaces where it comes from and nothing above it moves. Until then it is a
file in the client's asset directory with the shape this document already gave it, and the client
resolves a name through it: a catalogue id from the shelf, a file stem from a development flag,
and the same book either way.

The client fetches a book from wherever its shard says the shelf is, and from its own asset
directory when no shard has said. Three things that only showed up by running it:

- **`WebAssetPlugin` is already in `DefaultPlugins`** once the `https` feature is on, so it is
  `.set()` and not `.add_plugins()` — adding it is a plugin-already-added panic at startup.
- **`http` and `https` are separate features registering separate sources.** Production is
  HTTPS, and the development CDN is plain HTTP on `rocinante:3101`, so a desktop client
  rehearsing against it needs both. A browser could not load a book over plain HTTP from an
  HTTPS page anyway — mixed content — which [14-hosting.md](14-hosting.md) already covers.
- **`AssetMetaCheck::Never` is now needed in the desktop build too.** The browser build has
  always set it; on a filesystem the probe is a wasted stat, but a desktop client fetching a
  book over HTTP turns it into a round trip and a cached 404 per book. Running it against a
  stand-in CDN is what showed the `GET …/pg43-images-3.epub.meta 404` in the log.

Still to do here: the first decode of a plate happens on the frame it appears, which is a hitch
of tens of milliseconds on a page turn — worth moving to a task if it is ever felt, and not
worth the machinery before then.

Step 1 is in `crates/lc-books`: about 1 600 lines, no engine and no renderer, and its checks run
two ways. Twenty-two unit tests hold the invariants against a chapter written to break them, and
`--verify` runs the same invariants over whole books — 13 700 pages across six of them at three
frame sizes, in twelve seconds. The unit tests find the bug; the books find the case nobody
thought to write down, and on the first run they found two.

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
