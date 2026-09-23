//! The shelf, as the shard holds it.
//!
//! The catalog is a file the shard reads at boot and sends to every client; the bookmarks are
//! rows it keeps in memory and checkpoints beside the ships. Neither is part of the world:
//! nothing here is an event, nothing is cleared, and nothing a player does with a book changes
//! anything anyone else can see. See `lightcone/docs/21-library.md`.

use std::collections::HashMap;

use lc_proto::{Book, Bookmark, Writer};

/// Where the shelf is, when a shard was not told.
///
/// Empty rather than a guess: a shard with no base sends no library, and a client with no
/// library shows an empty shelf. Inventing a URL would have every client asking a host nobody
/// chose for files nobody published.
pub const NO_SHELF: &str = "";

#[derive(Default)]
pub struct Library {
    /// The prefix a book's `file` hangs off. One string, so the shelf can move CDNs without a
    /// client release — the same seam the site's `releases.cdn_base` is.
    pub base: String,
    pub books: Vec<Book>,
    /// Every account's bookmarks, most recently read first.
    ///
    /// The order **is** the recency: it is what "recently read" sorts by, and it is why nothing
    /// here carries a timestamp. Two machines agreeing about what "recent" means is a problem
    /// this feature does not have to have.
    marks: HashMap<String, Vec<Bookmark>>,
    /// Accounts whose bookmarks have changed since the last checkpoint.
    dirty: Vec<String>,
}

impl Library {
    /// Read a catalog in the format `books.toml` is written in.
    pub fn from_toml(base: &str, text: &str) -> Result<Self, String> {
        let catalog = lc_books::Catalog::from_toml(text).map_err(|e| e.to_string())?;
        let mut seen = std::collections::HashSet::new();
        for book in &catalog.books {
            if !seen.insert(book.id.clone()) {
                // Two rows under one id is a shelf where a bookmark means two things. Loudly at
                // boot, rather than quietly whenever someone opens the wrong one.
                return Err(format!("the catalog lists {} twice", book.id));
            }
            if book.file.contains('/') || book.file.contains('\\') || book.file.contains("..") {
                return Err(format!("{} names a file outside the shelf: {}", book.id, book.file));
            }
        }
        Ok(Self {
            base: base.to_owned(),
            books: catalog
                .books
                .into_iter()
                .map(|book| Book {
                    id: book.id,
                    title: book.title,
                    authors: book
                        .authors
                        .into_iter()
                        .map(|a| Writer { name: a.name, sort: a.sort })
                        .collect(),
                    year: book.year,
                    subjects: book.subjects,
                    file: book.file,
                })
                .collect(),
            ..Default::default()
        })
    }

    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }

    pub fn has(&self, book: &str) -> bool {
        self.books.iter().any(|b| b.id == book)
    }

    /// What to send an account on connecting.
    pub fn marks_for(&self, account: &str) -> Vec<Bookmark> {
        self.marks.get(account).cloned().unwrap_or_default()
    }

    /// Record where someone has got to.
    ///
    /// Refused for a book that is not on this shelf, which is the only check worth making: the
    /// offsets themselves cannot be verified without the epub, and nothing depends on them.
    pub fn set(&mut self, account: &str, mark: Bookmark) -> bool {
        if !self.has(&mark.book) {
            return false;
        }
        let marks = self.marks.entry(account.to_owned()).or_default();
        marks.retain(|m| m.book != mark.book);
        // To the front, because the front is what recency means here.
        marks.insert(0, mark);
        if !self.dirty.iter().any(|a| a == account) {
            self.dirty.push(account.to_owned());
        }
        true
    }

    /// Bookmarks written since the last time this was asked, and cleared by asking.
    pub fn take_dirty(&mut self) -> Vec<(String, Bookmark)> {
        let mut out = Vec::new();
        for account in std::mem::take(&mut self.dirty) {
            for mark in self.marks.get(&account).into_iter().flatten() {
                out.push((account.clone(), mark.clone()));
            }
        }
        out
    }

    /// Mark these accounts' shelves as needing writing again, because the write failed.
    pub fn redirty(&mut self, marks: &[(String, Bookmark)]) {
        for (account, _) in marks {
            // Once each: two rows for one shelf in a single upsert is an error.
            if !self.dirty.contains(account) {
                self.dirty.push(account.clone());
            }
        }
    }

    /// Adopt what was saved. Rows arrive most recently read first, which is the order kept.
    pub fn adopt(&mut self, marks: Vec<(String, Bookmark)>) {
        for (account, mark) in marks {
            self.marks.entry(account).or_default().push(mark);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELF: &str = r#"
        [[book]]
        id = "the-gilded-age"
        title = "The Gilded Age"
        file = "gilded.epub"

        [[book]]
        id = "a-princess-of-mars"
        title = "A Princess of Mars"
        file = "princess.epub"
    "#;

    fn mark(book: &str, at: u32) -> Bookmark {
        Bookmark { book: book.to_owned(), spine: 1, char_offset: at, location: at / 1024, locations: 400 }
    }

    #[test]
    fn a_catalog_with_one_id_twice_does_not_load() {
        let doubled = format!("{SHELF}\n[[book]]\nid = \"the-gilded-age\"\ntitle = \"Again\"\nfile = \"b.epub\"\n");
        assert!(Library::from_toml("https://cdn/", &doubled).is_err());
    }

    #[test]
    fn a_file_that_climbs_out_of_the_shelf_does_not_load() {
        let escaping = "[[book]]\nid = \"x\"\ntitle = \"X\"\nfile = \"../../etc/passwd\"\n";
        assert!(Library::from_toml("https://cdn/", escaping).is_err());
    }

    #[test]
    fn the_last_book_read_is_the_first_one_sent() {
        let mut shelf = Library::from_toml("https://cdn/", SHELF).unwrap();
        assert!(shelf.set("alice", mark("the-gilded-age", 10)));
        assert!(shelf.set("alice", mark("a-princess-of-mars", 20)));
        assert!(shelf.set("alice", mark("the-gilded-age", 30)));
        let sent = shelf.marks_for("alice");
        assert_eq!(sent.len(), 2, "one row per book, not one per page turn");
        assert_eq!(sent[0].book, "the-gilded-age");
        assert_eq!(sent[0].char_offset, 30);
        assert_eq!(sent[1].book, "a-princess-of-mars");
    }

    #[test]
    fn a_book_not_on_the_shelf_is_refused() {
        let mut shelf = Library::from_toml("https://cdn/", SHELF).unwrap();
        assert!(!shelf.set("alice", mark("something-else", 10)));
        assert!(shelf.marks_for("alice").is_empty());
    }

    #[test]
    fn accounts_do_not_see_each_others_places() {
        let mut shelf = Library::from_toml("https://cdn/", SHELF).unwrap();
        shelf.set("alice", mark("the-gilded-age", 10));
        shelf.set("bob", mark("the-gilded-age", 900));
        assert_eq!(shelf.marks_for("alice")[0].char_offset, 10);
        assert_eq!(shelf.marks_for("bob")[0].char_offset, 900);
    }

    #[test]
    fn only_what_changed_is_written_down() {
        let mut shelf = Library::from_toml("https://cdn/", SHELF).unwrap();
        shelf.set("alice", mark("the-gilded-age", 10));
        assert_eq!(shelf.take_dirty().len(), 1);
        assert!(shelf.take_dirty().is_empty(), "asking clears it");
        shelf.set("bob", mark("the-gilded-age", 20));
        let dirty = shelf.take_dirty();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].0, "bob");
    }
}
