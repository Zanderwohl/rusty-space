//! The shelf: a book as an asset, the face it is set in, and what is currently open.
//!
//! Going through the asset server rather than reading a file is what makes the browser build
//! and the desktop build the same code — and it is what will make the CDN a URL rather than a
//! port, when `lightcone/docs/21-library.md`'s step 2 lands. Nothing here knows about HTTP.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, LoadState};
use std::collections::HashMap;

use bevy::prelude::*;
use lc_books::{Block, Catalogue, Document, Epub, TocEntry};

/// Where a book is fetched from, relative to the asset root.
pub const SHELF: &str = "books";

/// The catalogue, which is the only thing that knows what is on the shelf.
///
/// A directory listing would do on a desktop and cannot exist in a browser, where the shelf is
/// a CDN prefix and HTTP has no way to ask what is under it. So the list is a file, which is
/// also what lets a title differ from a file name. Step 4 moves this to the server unchanged.
pub const CATALOGUE: &str = "books/books.toml";

/// The faces a page is set in: the family egui will know each by, and the file it comes from.
///
/// Loaded rather than compiled in, and only when a book is first opened: a player who never
/// opens one never pays for the 416 KB, which is what keeps the browser build's first-play
/// download where [14-hosting.md](../../lightcone/docs/14-hosting.md) measured it.
///
/// **Static cuts, not the variable files** — but no longer because egui cannot do better.
/// egui 0.36 rasterises through `skrifa` and shapes through `harfrust`, and both heed a
/// variation location; the `ab_glyph` limit this note used to cite went away with Bevy 0.19.
/// What the static cuts still buy is real italics and a real bold, against one file that would
/// have to be shipped at `wght` and shaped per run. See `lightcone/docs/21-library.md`.
pub const FACES: &[(&str, &str)] = &[
    (BODY, "fonts/Faustina-Regular.ttf"),
    (BODY_ITALIC, "fonts/Faustina-Italic.ttf"),
    (BODY_BOLD, "fonts/Faustina-Bold.ttf"),
    (BODY_BOLD_ITALIC, "fonts/Faustina-BoldItalic.ttf"),
    (DISPLAY, "fonts/LibreBaskerville-Regular.ttf"),
];

/// Faustina, for reading: a text face, and the one that has to hold up over forty minutes.
pub const BODY: &str = "reading";
pub const BODY_ITALIC: &str = "reading-italic";
pub const BODY_BOLD: &str = "reading-bold";
pub const BODY_BOLD_ITALIC: &str = "reading-bold-italic";
/// Libre Baskerville, for titles and headings: wider and heavier, which is what a line you
/// look at wants and what a page you read does not.
pub const DISPLAY: &str = "display";

#[derive(Asset, TypePath)]
pub struct Book {
    pub epub: Epub,
}

#[derive(Asset, TypePath)]
pub struct FontFace(pub Vec<u8>);

#[derive(Asset, TypePath)]
pub struct Shelved(pub Catalogue);

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Book(lc_books::Error),
    Catalogue(toml::de::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Book(e) => write!(f, "{e}"),
            Self::Catalogue(e) => write!(f, "the catalogue is malformed: {e}"),
        }
    }
}
impl std::error::Error for LoadError {}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Default, TypePath)]
pub struct BookLoader;

impl AssetLoader for BookLoader {
    type Asset = Book;
    type Settings = ();
    type Error = LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load: &mut LoadContext<'_>,
    ) -> Result<Book, LoadError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Epub::open(bytes)
            .map(|epub| Book { epub })
            .map_err(LoadError::Book)
    }

    fn extensions(&self) -> &[&str] {
        &["epub"]
    }
}

#[derive(Default, TypePath)]
pub struct CatalogueLoader;

impl AssetLoader for CatalogueLoader {
    type Asset = Shelved;
    type Settings = ();
    type Error = LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load: &mut LoadContext<'_>,
    ) -> Result<Shelved, LoadError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let text = String::from_utf8_lossy(&bytes);
        Catalogue::from_toml(&text)
            .map(Shelved)
            .map_err(LoadError::Catalogue)
    }

    fn extensions(&self) -> &[&str] {
        &["toml"]
    }
}

#[derive(Default, TypePath)]
pub struct FontLoader;

impl AssetLoader for FontLoader {
    type Asset = FontFace;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load: &mut LoadContext<'_>,
    ) -> Result<FontFace, std::io::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(FontFace(bytes))
    }

    fn extensions(&self) -> &[&str] {
        &["ttf", "otf"]
    }
}

/// What the reader has in its hands.
///
/// The parsed document is kept for one spine entry at a time. Keeping the whole book parsed
/// would be tens of megabytes of `String` for a novel with plates, to save a parse that takes
/// a few milliseconds and happens when a chapter is turned.
#[derive(Resource, Default)]
pub struct Shelf {
    /// Everything on the shelf, whether or not anything is open.
    pub catalogue: Catalogue,
    pub catalogue_handle: Option<Handle<Shelved>>,
    pub handle: Option<Handle<Book>>,
    /// The file this handle was asked for, so a change of book is noticed.
    pub file: Option<String>,
    pub title: String,
    pub chapters: Vec<TocEntry>,
    pub spine_count: usize,
    /// The parsed spine document, and which one it is.
    pub open: Option<(usize, Document)>,
    /// Characters in each spine document, filled one per frame after a book opens.
    ///
    /// A location is a fact about the **book**, not the chapter, so saying where someone is
    /// means knowing how long everything before them is. Parsing the whole book at once costs a
    /// few hundred milliseconds on a long one, which is a visible hitch on opening it; a chapter
    /// a frame is invisible and done inside a second.
    pub spine_chars: Vec<Option<usize>>,
    /// Where this account left off in each book, most recently read first. From the shard.
    pub marks: Vec<lc_proto::Bookmark>,
    /// What the shard says the shelf hangs off. Empty until it says.
    pub base: String,
    /// How big each plate in the open chapter is, in its own pixels.
    ///
    /// Read when the chapter is, because **pagination needs it**: a plate's height on the page
    /// follows from its shape, and a measurer that had to guess would move the text under the
    /// reader when the guess was corrected. Only the dimensions are kept; the pixels are read
    /// again when one is actually drawn.
    pub plates: HashMap<String, (u32, u32)>,
    pub trouble: Option<String>,
    pub face: Face,
}

/// How the reading faces are getting on.
///
/// Every face is optional and asked for together. A build missing one falls back a step — no
/// italic means the body sheared, no body at all means the interface font — so a shelf without
/// fonts is plainer and never broken.
#[derive(Default)]
pub enum Face {
    #[default]
    Unasked,
    Waiting(Vec<(&'static str, Handle<FontFace>)>),
    /// Whatever arrived has been installed into egui.
    Settled,
}

/// Fetch what the state says should be open, and parse the chapter it names.
pub fn keep_up(
    mut shelf: ResMut<Shelf>,
    mut state: ResMut<crate::app::Ui>,
    assets: Res<AssetServer>,
    mut books: ResMut<Assets<Book>>,
) {
    // A jump is spent here rather than in the reader, because what it changes is which chapter
    // is fetched. Spending it beside a page turn would turn pages in the chapter being left.
    if let Some((spine, offset)) = state.reading.goto.take() {
        state.reading.spine = spine;
        state.reading.offset = offset;
        state.reading.block = None;
        state.reading.asked = true;
    }
    let wanted = state.reading.book.clone();
    if wanted != shelf.file {
        *shelf = Shelf {
            face: std::mem::take(&mut shelf.face),
            catalogue: std::mem::take(&mut shelf.catalogue),
            catalogue_handle: shelf.catalogue_handle.clone(),
            marks: std::mem::take(&mut shelf.marks),
            base: std::mem::take(&mut shelf.base),
            ..Default::default()
        };
        shelf.file = wanted.clone();
        if let Some(name) = wanted {
            // The catalogue decides what a name means. Without one — a development build with
            // no shelf file — the name is taken for a file stem, which is what it used to be.
            let path = match shelf.catalogue.find(&name) {
                Some(entry) => shelf.where_to_fetch(&entry.file.clone()),
                None => Some(format!("{SHELF}/{name}.epub")),
            };
            match path {
                Some(path) => {
                    info!("opening {path}");
                    shelf.handle = Some(assets.load(path));
                }
                None => shelf.trouble = Some("that book is not on this shelf".to_owned()),
            }
        }
        return;
    }

    let Some(handle) = shelf.handle.clone() else {
        return;
    };
    if shelf.trouble.is_none()
        && matches!(assets.get_load_state(&handle), Some(LoadState::Failed(_)))
    {
        shelf.trouble = Some("that book is not on the shelf".to_owned());
        return;
    }
    let Some(mut book) = books.get_mut(&handle) else {
        return;
    };

    // The catalogue's title wins over the book's own: it is the one a person checked.
    if shelf.title.is_empty() {
        shelf.title = book.epub.title().to_owned();
        shelf.chapters = book.epub.toc().to_vec();
        shelf.spine_count = book.epub.spine().len();
        shelf.spine_chars = vec![None; shelf.spine_count];
    }

    let spine = state.reading.spine.min(shelf.spine_count.saturating_sub(1));
    if shelf.open.as_ref().map(|(at, _)| *at) != Some(spine) {
        match book.epub.document(spine) {
            Ok(doc) => {
                shelf.plates = plate_sizes(&mut book, &doc);
                if let Some(slot) = shelf.spine_chars.get_mut(spine) {
                    *slot = Some(doc.chars);
                }
                shelf.open = Some((spine, doc));
            }
            Err(why) => shelf.trouble = Some(why.to_string()),
        }
        return;
    }

    // One chapter a frame until the book's length is known. Parsed and thrown away: what is
    // wanted is the count, and keeping fifty chapters to save fifty parses would be tens of
    // megabytes of `String` for a number.
    if let Some(next) = shelf.spine_chars.iter().position(|c| c.is_none()) {
        let chars = book.epub.document(next).map(|doc| doc.chars).unwrap_or(0);
        shelf.spine_chars[next] = Some(chars);
    }
}

/// Whether a name is a file on the shelf rather than a way out of it.
///
/// One function and one test, which is the whole of what stands between a catalogue and an
/// asset loader. See `bevy_asset`'s own warning about loading URLs from elsewhere.
fn is_a_bare_name(file: &str) -> bool {
    !file.is_empty()
        && !file.contains('/')
        && !file.contains('\\')
        && !file.contains("..")
        && !file.starts_with('.')
}

/// The shape of every plate in a chapter.
///
/// A header read, not a decode: `into_dimensions` stops as soon as the format has told it how
/// big the image is, so this costs the zip entry rather than the picture.
fn plate_sizes(book: &mut Book, doc: &Document) -> HashMap<String, (u32, u32)> {
    let mut sizes = HashMap::new();
    for located in &doc.blocks {
        let Block::Image { path, .. } = &located.block else {
            continue;
        };
        if sizes.contains_key(path) {
            continue;
        }
        if let Ok(bytes) = book.epub.resource(path)
            && let Some(size) = dimensions(&bytes)
        {
            sizes.insert(path.clone(), size);
        }
    }
    sizes
}

/// How big an encoded image is, without decoding it.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

impl Shelf {
    /// Where to fetch a book from.
    ///
    /// **The client never receives a URL.** It receives a base from its own shard and a bare
    /// file name from the catalogue, and composes them here — so a shard that tried to point a
    /// client at somewhere else would have to do it with a file name, which this refuses. With
    /// no base the shelf is the asset directory, which is what a build with no shard has.
    pub fn where_to_fetch(&self, file: &str) -> Option<String> {
        if !is_a_bare_name(file) {
            return None;
        }
        if self.base.is_empty() {
            return Some(format!("{SHELF}/{file}"));
        }
        Some(format!("{}/{file}", self.base.trim_end_matches('/')))
    }

    /// Characters before a spine document, as far as the book has been measured.
    pub fn chars_before(&self, spine: usize) -> usize {
        self.spine_chars.iter().take(spine).filter_map(|c| *c).sum()
    }

    /// The whole book's length, once every chapter has been measured.
    pub fn total_chars(&self) -> Option<usize> {
        if self.spine_chars.is_empty() || self.spine_chars.iter().any(|c| c.is_none()) {
            return None;
        }
        Some(self.spine_chars.iter().flatten().sum())
    }

    /// Where a place in a chapter falls in the book, in locations.
    ///
    /// Counted from one, because a reader on the first page is at location 1 and not at
    /// location 0. `None` until the book has been measured.
    pub fn location(&self, spine: usize, char_offset: usize) -> Option<(u32, u32)> {
        let total = self.total_chars()?;
        let at = self.chars_before(spine) + char_offset;
        let of = total.div_ceil(lc_books::LOCATION_CHARS).max(1);
        Some(((at / lc_books::LOCATION_CHARS + 1) as u32, of as u32))
    }

    /// This account's place in a book, if it has one.
    pub fn mark_for(&self, book: &str) -> Option<&lc_proto::Bookmark> {
        self.marks.iter().find(|m| m.book == book)
    }

    /// Books this account has read, most recently first.
    pub fn recent(&self) -> Vec<String> {
        self.marks.iter().map(|m| m.book.clone()).collect()
    }

    /// The catalogue id of the open book, whatever name it was opened under.
    ///
    /// A development flag names a book by its file and the shelf names it by its id; both end
    /// up here, and the shelf marks the right row either way.
    pub fn open_id(&self) -> Option<&str> {
        let name = self.file.as_deref()?;
        self.catalogue.find(name).map(|entry| entry.id.as_str())
    }
}

/// Where a chapter begins, worked out when it is asked for.
///
/// Not at load: resolving every entry means parsing every document it points into, and one book
/// on the first shelf has seventy-three of them.
pub fn chapter_start(
    books: &mut Assets<Book>,
    shelf: &Shelf,
    entry: &TocEntry,
) -> Option<(usize, usize)> {
    let handle = shelf.handle.as_ref()?;
    let mut book = books.get_mut(handle)?;
    book.epub.locate(entry).map(|at| (at.spine, at.char_offset))
}

/// Take the shelf the shard sent, and the places it kept for this account.
///
/// The shard's word replaces the file's. A shipped client is told what there is to read by the
/// world it is reading in; the file is what a single-process build has instead of a shard.
pub fn take_from_shard(mut shelf: ResMut<Shelf>, mut uplink: ResMut<crate::uplink::Uplink>) {
    if let Some((base, books)) = uplink.shelf.take() {
        shelf.base = base;
        shelf.catalogue = lc_books::Catalogue {
            books: books
                .into_iter()
                .map(|b| lc_books::catalogue::Entry {
                    id: b.id,
                    title: b.title,
                    authors: b
                        .authors
                        .into_iter()
                        .map(|a| lc_books::catalogue::Writer {
                            name: a.name,
                            sort: a.sort,
                        })
                        .collect(),
                    year: b.year,
                    subjects: b.subjects,
                    file: b.file,
                    sha256: None,
                    source: None,
                })
                .collect(),
        };
        info!("the shard lends {} books", shelf.catalogue.books.len());
    }
    if let Some(marks) = uplink.bookmarks.take() {
        shelf.marks = marks;
    }
}

/// When a place is written down: at once when the player moved, and on an interval otherwise.
///
/// A function over state rather than a system, because the rule it encodes — *a page turn, a
/// jump, the first sight of a book and putting one down all report immediately; a page that
/// merely reflowed under a resize waits* — is the part worth being sure of, and a system
/// carrying a socket is the part that cannot be tested.
///
/// The interval is a backstop rather than the rule. A place only moves when the player moves it
/// — [`crate::reader`] does not write one down for a reflow — so in practice every change is a
/// deliberate one and goes at once. What the interval stops is a future path that moves the
/// offset without anybody asking: every inbound message is charged against a connection's budget,
/// two a second sustained and thirty in a burst, and something changing it every frame would
/// spend the lot and then be throttled, which is how a bookmark gets lost by trying too hard to
/// save it.
#[derive(Default)]
pub struct Reporter {
    pending: Option<lc_proto::Bookmark>,
    sent: Option<lc_proto::Bookmark>,
    quiet_for: f32,
}

impl Reporter {
    /// Advance a frame. `here` is where the reader is now, when there is a book open and its
    /// length is known; `open` is the book open at this instant, which is how putting one down
    /// is told from reading it.
    pub fn tick(
        &mut self,
        dt: f32,
        open: Option<&str>,
        here: Option<lc_proto::Bookmark>,
        asked: bool,
    ) -> Option<lc_proto::Bookmark> {
        self.quiet_for += dt;
        // A place held for a book that is no longer the open one goes now, and **before
        // anything can take its place**: the book was closed or another was opened over it, no
        // later position is coming, and the interval that exists to spare the shard a message a
        // page turn must not be what loses the last one.
        if let Some(mark) = self.pending.clone()
            && open != Some(mark.book.as_str())
        {
            return Some(self.flush(mark));
        }
        if let Some(mark) = here
            && self.sent.as_ref() != Some(&mark)
        {
            self.pending = Some(mark);
        }
        let mark = self.pending.clone()?;
        // The first word about a book does not wait either: opening one at chapter nine and
        // being disconnected four seconds later should not record chapter one.
        let first = self.sent.as_ref().map(|s| s.book.as_str()) != Some(mark.book.as_str());
        if !asked && !first && self.quiet_for < REPORT_EVERY_S {
            return None;
        }
        Some(self.flush(mark))
    }

    fn flush(&mut self, mark: lc_proto::Bookmark) -> lc_proto::Bookmark {
        self.sent = Some(mark.clone());
        self.pending = None;
        self.quiet_for = 0.0;
        mark
    }
}

/// Tell the shard where the player has got to.
pub fn report_place(
    mut state: ResMut<crate::app::Ui>,
    mut shelf: ResMut<Shelf>,
    mut uplink: ResMut<crate::uplink::Uplink>,
    time: Res<Time>,
    mut reporter: Local<Reporter>,
) {
    let open = shelf.open_id().map(str::to_owned);
    // Until the book has been measured there is no honest location to report, and a bookmark
    // with the wrong one would be written down and shown as a percentage of nothing.
    let here = open.as_ref().and_then(|book| {
        let (location, locations) = shelf.location(state.reading.spine, state.reading.offset)?;
        Some(lc_proto::Bookmark {
            book: book.clone(),
            spine: state.reading.spine as u32,
            char_offset: state.reading.offset as u32,
            location,
            locations,
        })
    });
    let asked = std::mem::take(&mut state.reading.asked);
    let Some(mark) = reporter.tick(time.delta_secs(), open.as_deref(), here, asked) else {
        return;
    };

    uplink.say(lc_proto::Inbound::SetReading(mark.clone()));
    // Kept here as well as sent. The shard states bookmarks once, on connecting, so a shelf that
    // waited to be told would show yesterday's place for the book just put down.
    shelf.marks.retain(|m| m.book != mark.book);
    shelf.marks.insert(0, mark);
}

/// How often a place is reported while it keeps changing.
const REPORT_EVERY_S: f32 = 5.0;

/// Fetch the catalogue once, and keep it where everything can read it.
///
/// **Not in the browser**, where there is no file to fetch: `tools/build-wasm.sh` stages the
/// fonts and the sky and not the shelf, because 43 MB of epub belongs on the CDN once rather
/// than under every build id. A browser build is always told what to read by its shard, so
/// asking anyway only bought an asset-server error in the console on every page load.
pub fn read_catalogue(
    mut shelf: ResMut<Shelf>,
    assets: Res<AssetServer>,
    shelved: Res<Assets<Shelved>>,
) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    match &shelf.catalogue_handle {
        None => shelf.catalogue_handle = Some(assets.load(CATALOGUE)),
        Some(handle) => {
            if shelf.catalogue.books.is_empty()
                && let Some(loaded) = shelved.get(handle)
            {
                shelf.catalogue = loaded.0.clone();
                info!("the shelf holds {} books", shelf.catalogue.books.len());
            }
        }
    }
}

pub struct LibraryPlugin;

impl Plugin for LibraryPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Book>()
            .init_asset::<FontFace>()
            .init_asset::<Shelved>()
            .init_asset_loader::<BookLoader>()
            .init_asset_loader::<FontLoader>()
            .init_asset_loader::<CatalogueLoader>()
            .init_resource::<Shelf>()
            .add_systems(
                Update,
                (read_catalogue, take_from_shard, keep_up, report_place).chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shelf_with(base: &str) -> Shelf {
        Shelf {
            base: base.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn with_no_base_a_book_comes_from_the_asset_directory() {
        let shelf = shelf_with("");
        assert_eq!(
            shelf.where_to_fetch("A Princess of Mars.epub").as_deref(),
            Some("books/A Princess of Mars.epub")
        );
    }

    #[test]
    fn a_base_is_a_prefix_whether_or_not_it_ends_in_a_slash() {
        for base in [
            "https://cdn.example/library",
            "https://cdn.example/library/",
        ] {
            assert_eq!(
                shelf_with(base).where_to_fetch("gilded.epub").as_deref(),
                Some("https://cdn.example/library/gilded.epub"),
            );
        }
    }

    fn at(book: &str, offset: u32) -> lc_proto::Bookmark {
        lc_proto::Bookmark {
            book: book.to_owned(),
            spine: 1,
            char_offset: offset,
            location: offset / 1024 + 1,
            locations: 400,
        }
    }

    #[test]
    fn a_steady_reader_costs_one_message_an_interval() {
        let mut reporter = Reporter::default();
        // The first change goes at once; the reader has said something new and nothing is owed.
        assert_eq!(
            reporter.tick(0.016, Some("a"), Some(at("a", 10)), false),
            Some(at("a", 10))
        );
        // Turning pages inside the interval says nothing.
        for page in 1..20 {
            assert_eq!(
                reporter.tick(0.2, Some("a"), Some(at("a", page * 1000)), false),
                None
            );
        }
        // And then the latest of them, once.
        assert_eq!(
            reporter.tick(2.0, Some("a"), Some(at("a", 19_000)), false),
            Some(at("a", 19_000))
        );
    }

    #[test]
    fn a_page_turn_is_written_down_the_moment_it_happens() {
        let mut reporter = Reporter::default();
        reporter.tick(0.016, Some("a"), Some(at("a", 10)), true);
        // Three turns inside a second. A crash after any of them should cost nothing.
        for page in 1..4 {
            let at_page = at("a", page * 1_400);
            assert_eq!(
                reporter.tick(0.3, Some("a"), Some(at_page.clone()), true),
                Some(at_page),
                "a turned page waited for the interval",
            );
        }
    }

    #[test]
    fn a_page_that_only_reflowed_waits() {
        let mut reporter = Reporter::default();
        reporter.tick(0.016, Some("a"), Some(at("a", 10)), true);
        // A resize drag: the same sentence, landing a few characters along, every frame.
        for frame in 1..60 {
            assert_eq!(
                reporter.tick(0.016, Some("a"), Some(at("a", 10 + frame)), false),
                None
            );
        }
    }

    #[test]
    fn putting_a_book_down_does_not_wait_for_the_interval() {
        let mut reporter = Reporter::default();
        reporter.tick(0.016, Some("a"), Some(at("a", 10)), false);
        // A page turn, then the shelf button, well inside the interval.
        assert_eq!(
            reporter.tick(0.2, Some("a"), Some(at("a", 4_000)), false),
            None
        );
        assert_eq!(
            reporter.tick(0.2, None, None, false),
            Some(at("a", 4_000)),
            "the last page read was lost when the book was closed",
        );
        assert_eq!(
            reporter.tick(9.0, None, None, false),
            None,
            "and it is not sent twice"
        );
    }

    #[test]
    fn opening_another_book_flushes_the_one_before_it() {
        let mut reporter = Reporter::default();
        reporter.tick(0.016, Some("a"), Some(at("a", 10)), false);
        reporter.tick(0.2, Some("a"), Some(at("a", 5_000)), false);
        assert_eq!(
            reporter.tick(0.2, Some("b"), Some(at("b", 0)), false),
            Some(at("a", 5_000))
        );
    }

    #[test]
    fn a_book_whose_length_is_not_known_yet_is_not_reported() {
        let mut reporter = Reporter::default();
        // Measuring takes a frame per chapter, and a location computed before it finishes would
        // be a percentage of a book that is still being counted.
        for _ in 0..40 {
            assert_eq!(reporter.tick(0.2, Some("a"), None, false), None);
        }
    }

    #[test]
    fn a_file_name_that_is_not_a_file_name_fetches_nothing() {
        let shelf = shelf_with("https://cdn.example/library/");
        for name in [
            "../../etc/passwd",
            "sub/dir.epub",
            "https://elsewhere/x.epub",
            "",
            ".hidden",
        ] {
            assert_eq!(shelf.where_to_fetch(name), None, "{name} was let through");
        }
    }
}
