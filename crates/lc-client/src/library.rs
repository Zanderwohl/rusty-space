//! The shelf: a book as an asset, the face it is set in, and what is currently open.
//!
//! Going through the asset server rather than reading a file is what makes the browser build
//! and the desktop build the same code — and it is what will make the CDN a URL rather than a
//! port, when `lightcone/docs/19-library.md`'s step 2 lands. Nothing here knows about HTTP.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, LoadState};
use bevy::prelude::*;
use lc_books::{Document, Epub, TocEntry};

/// Where a book is fetched from, relative to the asset root.
pub const SHELF: &str = "books";

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

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Book(lc_books::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Book(e) => write!(f, "{e}"),
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
    pub handle: Option<Handle<Book>>,
    /// The file this handle was asked for, so a change of book is noticed.
    pub file: Option<String>,
    pub title: String,
    pub chapters: Vec<TocEntry>,
    pub spine_count: usize,
    /// The parsed spine document, and which one it is.
    pub open: Option<(usize, Document)>,
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
        *shelf = Shelf { face: std::mem::take(&mut shelf.face), ..Default::default() };
        shelf.file = wanted.clone();
        if let Some(file) = wanted {
            shelf.handle = Some(assets.load(format!("{SHELF}/{file}.epub")));
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

    if shelf.title.is_empty() {
        shelf.title = book.epub.title().to_owned();
        shelf.chapters = book.epub.toc().to_vec();
        shelf.spine_count = book.epub.spine().len();
    }

    let spine = state.reading.spine.min(shelf.spine_count.saturating_sub(1));
    if shelf.open.as_ref().map(|(at, _)| *at) != Some(spine) {
        match book.epub.document(spine) {
            Ok(doc) => shelf.open = Some((spine, doc)),
            Err(why) => shelf.trouble = Some(why.to_string()),
        }
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

pub struct LibraryPlugin;

impl Plugin for LibraryPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Book>()
            .init_asset::<FontFace>()
            .init_asset_loader::<BookLoader>()
            .init_asset_loader::<FontLoader>()
            .init_resource::<Shelf>()
            .add_systems(Update, keep_up);
    }
}
