//! Epubs, for the Lightcone library.
//!
//! No engine and no renderer: a zip, some XML, a document model and the coordinate a reader's
//! place is kept in. What this crate deliberately does not do is lay anything out — only the
//! client can measure the client's fonts, so pagination is a function over a `Measure` supplied
//! by whoever is drawing. See `lightcone/docs/21-library.md`.

#![forbid(unsafe_code)]

mod archive;
pub mod catalogue;
pub mod grid;
pub mod nav;
pub mod opf;
pub mod paginate;
pub mod text;
mod xml;

pub use catalogue::{Catalogue, Entry, Order};
pub use grid::Grid;
pub use nav::TocEntry;
pub use opf::{Author, Item, Metadata};
pub use paginate::{Cursor, Frame, Measure, Measured, Page, Row, Slice};
pub use text::{Block, Document, Located, Run, Style, Text};

/// Characters of body text in one location.
///
/// A page is a fact about a window and cannot be written down; a location is a fact about the
/// book. The number is arbitrary and fixed forever — changing it moves every bookmark ever
/// saved, which is a migration nobody can perform because the text is not on the server.
pub const LOCATION_CHARS: usize = 1024;

/// Where a reader is: which spine document, and how far into its text.
///
/// **Not a page.** Pages depend on the window, the font and the build that laid them out; this
/// survives all three, and a phone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Locator {
    pub spine: usize,
    pub char_offset: usize,
}

#[derive(Debug)]
pub enum Error {
    Zip(String),
    Xml(String),
    Missing(String),
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Zip(e) => write!(f, "not a readable epub: {e}"),
            Error::Xml(e) => write!(f, "malformed XML: {e}"),
            Error::Missing(path) => write!(f, "the book does not contain {path}"),
            Error::Malformed(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for Error {}

pub struct Epub {
    archive: archive::Archive,
    package: opf::Package,
    toc: Vec<TocEntry>,
}

impl Epub {
    pub fn open(bytes: Vec<u8>) -> Result<Self, Error> {
        let mut archive = archive::Archive::open(bytes)?;
        let container = archive.read_text("META-INF/container.xml")?;
        let opf_path = opf::root_path(&container)?;
        let package = opf::parse(&archive.read_text(&opf_path)?, &opf_path)?;
        let toc = read_toc(&mut archive, &package);
        Ok(Self { archive, package, toc })
    }

    pub fn metadata(&self) -> &Metadata {
        &self.package.metadata
    }

    pub fn title(&self) -> &str {
        &self.package.metadata.title
    }

    pub fn authors(&self) -> &[Author] {
        &self.package.metadata.authors
    }

    pub fn toc(&self) -> &[TocEntry] {
        &self.toc
    }

    /// The spine: every document, in reading order.
    pub fn spine(&self) -> Vec<&Item> {
        self.package.documents()
    }

    pub fn cover(&self) -> Option<&Item> {
        self.package.cover()
    }

    /// One spine document, parsed.
    ///
    /// Parsed on demand and not cached here: the caller knows which chapters are worth keeping
    /// and this crate does not.
    pub fn document(&mut self, spine: usize) -> Result<Document, Error> {
        let path = self
            .package
            .documents()
            .get(spine)
            .map(|i| i.path.clone())
            .ok_or_else(|| Error::Missing(format!("spine document {spine}")))?;
        let xhtml = self.archive.read_text(&path)?;
        Ok(text::parse(&xhtml, &path))
    }

    /// Raw bytes of a resource — an image, usually — by archive path.
    pub fn resource(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        self.archive.read(path)
    }

    /// Which spine document a table-of-contents entry lands in.
    pub fn spine_of(&self, path: &str) -> Option<usize> {
        self.package.documents().iter().position(|i| i.path == path)
    }

    /// Where a table-of-contents entry actually starts.
    ///
    /// Parses the document it lands in, because a fragment's offset is a fact about the text and
    /// there is nowhere else to learn it. The spine index alone is not the answer: one book on
    /// the first shelf has seventy-three chapters in nine documents.
    pub fn locate(&mut self, entry: &TocEntry) -> Option<Locator> {
        let spine = self.spine_of(&entry.path)?;
        let Some(fragment) = entry.fragment.as_deref() else {
            return Some(Locator { spine, char_offset: 0 });
        };
        let doc = self.document(spine).ok()?;
        // A fragment naming nothing is a broken link in the book, and the head of the document
        // is where a reader following it should still end up.
        Some(Locator { spine, char_offset: doc.anchor(fragment).unwrap_or(0) })
    }
}

fn read_toc(archive: &mut archive::Archive, package: &opf::Package) -> Vec<TocEntry> {
    // EPUB 3 first: a book carrying both is a book converted from EPUB 2, and the nav is the one
    // its publisher looked at.
    if let Some(item) = package.nav()
        && let Ok(xhtml) = archive.read_text(&item.path)
    {
        let entries = nav::from_nav(&xhtml, &item.path);
        if !entries.is_empty() {
            return entries;
        }
    }
    if let Some(item) = package.ncx.as_deref().and_then(|id| package.item(id))
        && let Ok(xml) = archive.read_text(&item.path)
    {
        return nav::from_ncx(&xml, &item.path);
    }
    Vec::new()
}
