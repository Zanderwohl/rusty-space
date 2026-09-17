//! What is on the shelf.
//!
//! Written by a person and read by everyone: the titles, the authors and the subjects a player
//! searches by, and the file name each one is fetched under. **The file name is not the
//! identity.** The first real shelf held `pg2488-images-3.epub` and two files whose names had
//! lost a colon, which is the whole reason this exists rather than a directory listing.
//!
//! The server owns the file and sends it; this crate owns its shape, so both ends agree by
//! construction. See `lightcone/docs/19-library.md`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Catalogue {
    #[serde(default, rename = "book")]
    pub books: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Entry {
    /// Stable, and never derived from the file name at read time.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub authors: Vec<Writer>,
    /// First publication, which no book on the shelf knows about itself: an epub states the day
    /// its transcription was posted. Absent where nobody has looked it up.
    #[serde(default)]
    pub year: Option<i32>,
    /// What it is about, in the words the transcription used. Searched, never displayed as a
    /// category, because these are library subject headings and not genres.
    #[serde(default)]
    pub subjects: Vec<String>,
    pub file: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Writer {
    pub name: String,
    /// How the name files. `Twain, Mark` and not `Mark Twain`, which is the difference between
    /// a shelf sorted by author and a shelf sorted by first name.
    #[serde(default)]
    pub sort: Option<String>,
}

impl Writer {
    pub fn sort_key(&self) -> &str {
        self.sort
            .as_deref()
            .unwrap_or_else(|| self.name.rsplit(' ').next().unwrap_or(&self.name))
    }
}

impl Catalogue {
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.books.iter().find(|b| b.id == id)
    }

    /// An entry by id, or failing that by the stem of its file name.
    ///
    /// The second is for a development flag, where `--book pg43-images-3` is a name someone can
    /// type and `--book the-strange-case-of-dr-jekyll-and-mr-hyde` is not.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.get(name).or_else(|| {
            self.books.iter().find(|b| b.file.rsplit_once('.').map(|(s, _)| s) == Some(name))
        })
    }
}

impl Entry {
    /// Everything this book can be found by, folded once so a filter does not fold it per word.
    pub fn haystack(&self) -> String {
        let mut hay = self.title.to_lowercase();
        for author in &self.authors {
            hay.push(' ');
            hay.push_str(&author.name.to_lowercase());
            if let Some(sort) = &author.sort {
                hay.push(' ');
                hay.push_str(&sort.to_lowercase());
            }
        }
        for subject in &self.subjects {
            hay.push(' ');
            hay.push_str(&subject.to_lowercase());
        }
        if let Some(year) = self.year {
            hay.push(' ');
            hay.push_str(&year.to_string());
        }
        hay
    }

    /// The title as it files: without the article it happens to start with.
    ///
    /// `The Gilded Age` belongs under G. Every library in the world does this and a shelf that
    /// does not has a third of its stock under T.
    pub fn sort_title(&self) -> &str {
        let title = self.title.trim();
        for article in ["The ", "A ", "An "] {
            if let Some(rest) = title.strip_prefix(article) {
                return rest;
            }
        }
        title
    }

    pub fn sort_author(&self) -> &str {
        self.authors.first().map(|a| a.sort_key()).unwrap_or("")
    }

    pub fn by_line(&self) -> String {
        match self.authors.len() {
            0 => String::new(),
            1 => self.authors[0].name.clone(),
            _ => {
                let names: Vec<&str> = self.authors.iter().map(|a| a.name.as_str()).collect();
                let last = names.len() - 1;
                format!("{} and {}", names[..last].join(", "), names[last])
            }
        }
    }
}

/// Whether a book answers to what was typed.
///
/// Every word has to appear somewhere — title, author, filing name, subject or year — as a
/// partial match, in any order and in any case. So `twain miss` finds the Mississippi novels
/// and `verne sea` finds the one about the sea, which is how anyone actually looks for a book
/// they have half-remembered.
pub fn matches(entry: &Entry, query: &str) -> bool {
    let hay = entry.haystack();
    query.split_whitespace().all(|word| hay.contains(&word.to_lowercase()))
}

/// How the shelf is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Title,
    Author,
    Year,
}

impl Order {
    pub const ALL: [Order; 3] = [Order::Title, Order::Author, Order::Year];

    pub fn label(&self) -> &'static str {
        match self {
            Order::Title => "title",
            Order::Author => "author",
            Order::Year => "year",
        }
    }
}

/// The entries that match, in the order asked for.
///
/// Sorting is case-insensitive and falls back to the filing title, so two books by one author
/// are not in whatever order the file happened to list them.
pub fn shelve<'a>(catalogue: &'a Catalogue, query: &str, order: Order) -> Vec<&'a Entry> {
    let mut found: Vec<&Entry> =
        catalogue.books.iter().filter(|b| matches(b, query)).collect();
    found.sort_by(|a, b| {
        let title = |e: &Entry| e.sort_title().to_lowercase();
        match order {
            Order::Title => title(a).cmp(&title(b)),
            Order::Author => a
                .sort_author()
                .to_lowercase()
                .cmp(&b.sort_author().to_lowercase())
                .then_with(|| title(a).cmp(&title(b))),
            // Undated books file last rather than in 1970, because a missing year is not a year.
            Order::Year => a
                .year
                .unwrap_or(i32::MAX)
                .cmp(&b.year.unwrap_or(i32::MAX))
                .then_with(|| title(a).cmp(&title(b))),
        }
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shelf() -> Catalogue {
        Catalogue::from_toml(
            r#"
            [[book]]
            id = "the-gilded-age"
            title = "The Gilded Age: A Tale of Today"
            authors = [
                { name = "Mark Twain", sort = "Twain, Mark" },
                { name = "Charles Dudley Warner", sort = "Warner, Charles Dudley" },
            ]
            year = 1873
            subjects = ["Satire", "Politics and government -- Fiction"]
            file = "The Gilded Age A Tale of Today.epub"

            [[book]]
            id = "a-princess-of-mars"
            title = "A Princess of Mars"
            authors = [{ name = "Edgar Rice Burroughs", sort = "Burroughs, Edgar Rice" }]
            year = 1912
            subjects = ["Science fiction", "Mars (Planet) -- Fiction"]
            file = "A Princess of Mars.epub"

            [[book]]
            id = "twenty-thousand-leagues-under-the-seas"
            title = "Twenty Thousand Leagues Under the Seas"
            authors = [{ name = "Jules Verne", sort = "Verne, Jules" }]
            subjects = ["Sea stories", "Submarines (Ships) -- Fiction"]
            file = "pg2488-images-3.epub"
            "#,
        )
        .expect("the fixture parses")
    }

    #[test]
    fn a_title_files_under_its_first_real_word() {
        let shelf = shelf();
        let order: Vec<&str> =
            shelve(&shelf, "", Order::Title).iter().map(|e| e.title.as_str()).collect();
        assert_eq!(
            order,
            [
                "The Gilded Age: A Tale of Today",
                "A Princess of Mars",
                "Twenty Thousand Leagues Under the Seas"
            ],
            "G, P, T — not A, A, T"
        );
    }

    #[test]
    fn author_order_is_the_filing_name() {
        let shelf = shelf();
        let order: Vec<&str> =
            shelve(&shelf, "", Order::Author).iter().map(|e| e.sort_author()).collect();
        assert_eq!(order, ["Burroughs, Edgar Rice", "Twain, Mark", "Verne, Jules"]);
    }

    #[test]
    fn a_book_with_no_year_files_last_rather_than_first() {
        let shelf = shelf();
        let order: Vec<Option<i32>> =
            shelve(&shelf, "", Order::Year).iter().map(|e| e.year).collect();
        assert_eq!(order, [Some(1873), Some(1912), None]);
    }

    #[test]
    fn every_word_has_to_land_somewhere_and_case_does_not_count() {
        let shelf = shelf();
        let found = |q: &str| -> Vec<&str> {
            shelve(&shelf, q, Order::Title).iter().map(|e| e.id.as_str()).collect()
        };
        assert_eq!(found("TWAIN"), ["the-gilded-age"], "case does not count");
        assert_eq!(found("sea"), ["twenty-thousand-leagues-under-the-seas"], "a subject");
        assert_eq!(found("warner gilded"), ["the-gilded-age"], "two words, two fields");
        assert_eq!(found("1912"), ["a-princess-of-mars"], "a year");
        assert_eq!(found("princ"), ["a-princess-of-mars"], "a partial word");
        assert!(found("twain mars").is_empty(), "every word has to land");
    }

    #[test]
    fn a_development_flag_can_name_a_book_by_its_file() {
        let shelf = shelf();
        assert_eq!(shelf.find("pg2488-images-3").map(|e| e.id.as_str()), Some("twenty-thousand-leagues-under-the-seas"));
        assert_eq!(shelf.find("a-princess-of-mars").map(|e| e.title.as_str()), Some("A Princess of Mars"));
        assert!(shelf.find("nothing-like-this").is_none());
    }

    #[test]
    fn two_authors_read_as_a_sentence() {
        let shelf = shelf();
        assert_eq!(shelf.get("the-gilded-age").unwrap().by_line(), "Mark Twain and Charles Dudley Warner");
    }
}
