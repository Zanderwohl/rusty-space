//! The shelf: a book as an asset, the face it is set in, and what is currently open.
//!
//! Going through the asset server rather than reading a file is what makes the browser build
//! and the desktop build the same code — and it is what will make the CDN a URL rather than a
//! port, when `lightcone/docs/19-library.md`'s step 2 lands. Nothing here knows about HTTP.

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

/// The face the page is set in, if the build ships one.
///
/// Loaded rather than compiled in, and only when a book is first opened: a player who never
/// opens one never pays for it, which is what keeps the browser build's first-play download
/// where [14-hosting.md](../../lightcone/docs/14-hosting.md) measured it.
pub const READING_FACE: &str = "fonts/reader.ttf";

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

#[derive(Default)]
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
        Epub::open(bytes).map(|epub| Book { epub }).map_err(LoadError::Book)
    }

    fn extensions(&self) -> &[&str] {
        &["epub"]
    }
}

#[derive(Default)]
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
        Catalogue::from_toml(&text).map(Shelved).map_err(LoadError::Catalogue)
    }

    fn extensions(&self) -> &[&str] {
        &["toml"]
    }
}

#[derive(Default)]
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

/// How the reading face is getting on.
#[derive(Default, PartialEq, Eq)]
pub enum Face {
    #[default]
    Unasked,
    Waiting(Handle<FontFace>),
    /// Installed into egui, or absent and the interface font is standing in for it.
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
                Some(entry) => format!("{SHELF}/{}", entry.file),
                None => format!("{SHELF}/{name}.epub"),
            };
            shelf.handle = Some(assets.load(path));
        }
        return;
    }

    let Some(handle) = shelf.handle.clone() else { return };
    if shelf.trouble.is_none()
        && matches!(assets.get_load_state(&handle), Some(LoadState::Failed(_)))
    {
        shelf.trouble = Some("that book is not on the shelf".to_owned());
        return;
    }
    let Some(book) = books.get_mut(&handle) else { return };

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
                shelf.plates = plate_sizes(book, &doc);
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

/// The shape of every plate in a chapter.
///
/// A header read, not a decode: `into_dimensions` stops as soon as the format has told it how
/// big the image is, so this costs the zip entry rather than the picture.
fn plate_sizes(book: &mut Book, doc: &Document) -> HashMap<String, (u32, u32)> {
    let mut sizes = HashMap::new();
    for located in &doc.blocks {
        let Block::Image { path, .. } = &located.block else { continue };
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
pub fn chapter_start(books: &mut Assets<Book>, shelf: &Shelf, entry: &TocEntry) -> Option<(usize, usize)> {
    let handle = shelf.handle.as_ref()?;
    let book = books.get_mut(handle)?;
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
                        .map(|a| lc_books::catalogue::Writer { name: a.name, sort: a.sort })
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

/// Tell the shard where the player has got to.
///
/// Debounced: a page turn every few seconds must not be a message every few seconds. A change is
/// sent at once and then no more often than this, which means the common case — reading steadily
/// — costs one small message a minute and stopping mid-page still records where you stopped.
pub fn report_place(
    state: Res<crate::app::Ui>,
    mut shelf: ResMut<Shelf>,
    mut uplink: ResMut<crate::uplink::Uplink>,
    time: Res<Time>,
    mut sent: Local<Option<lc_proto::Bookmark>>,
    mut quiet_for: Local<f32>,
) {
    *quiet_for += time.delta_secs();
    let Some(book) = shelf.open_id().map(str::to_owned) else { return };
    let spine = state.reading.spine;
    let offset = state.reading.offset;
    // Until the book has been measured there is no honest location to report, and a bookmark
    // with the wrong one would be written down and shown as a percentage of nothing.
    let Some((location, locations)) = shelf.location(spine, offset) else { return };

    let mark = lc_proto::Bookmark {
        book,
        spine: spine as u32,
        char_offset: offset as u32,
        location,
        locations,
    };
    if sent.as_ref() == Some(&mark) {
        return;
    }
    // A different book is a different fact and does not wait its turn.
    let same_book = sent.as_ref().is_some_and(|s| s.book == mark.book);
    if same_book && *quiet_for < REPORT_EVERY_S {
        return;
    }
    uplink.say(lc_proto::Inbound::SetReading(mark.clone()));
    // Kept here as well as sent. The shard states bookmarks once, on connecting, so a shelf
    // that waited to be told would show yesterday's place for the book being read right now.
    shelf.marks.retain(|m| m.book != mark.book);
    shelf.marks.insert(0, mark.clone());
    *sent = Some(mark);
    *quiet_for = 0.0;
}

/// How often a place is reported while it keeps changing.
const REPORT_EVERY_S: f32 = 5.0;

/// Fetch the catalogue once, and keep it where everything can read it.
pub fn read_catalogue(
    mut shelf: ResMut<Shelf>,
    assets: Res<AssetServer>,
    shelved: Res<Assets<Shelved>>,
) {
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
            .add_systems(Update, (read_catalogue, take_from_shard, keep_up, report_place).chain());
    }
}
