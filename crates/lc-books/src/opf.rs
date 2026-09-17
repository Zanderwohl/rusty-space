//! The package document: what the book is, what it contains, and the order to read it in.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::Error;
use crate::xml::{attr, local, unescape};

#[derive(Clone, Debug, Default)]
pub struct Author {
    pub name: String,
    /// How the name files, when the book says. `Shelley` and not `Mary Wollstonecraft Shelley`
    /// — which is the whole reason the shelf can sort by author at all.
    pub sort: Option<String>,
}

impl Author {
    /// The key to sort this author under, guessed when the book did not say.
    ///
    /// The guess is the last whitespace-separated word, which is right for most Western names
    /// and wrong often enough that [`Author::sort`] has to exist to override it.
    pub fn sort_key(&self) -> &str {
        self.sort
            .as_deref()
            .unwrap_or_else(|| self.name.rsplit(' ').next().unwrap_or(&self.name))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub title: String,
    pub authors: Vec<Author>,
    pub language: Option<String>,
    /// As written in the file, which is **not** the year of first publication: Gutenberg writes
    /// the date it posted the transcription. See `lightcone/docs/19-library.md`.
    pub date: Option<String>,
    pub identifier: Option<String>,
    pub subjects: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    /// Resolved against the archive root, not the href the manifest wrote.
    pub path: String,
    pub media_type: String,
    pub properties: String,
}

#[derive(Clone, Debug, Default)]
pub struct Package {
    pub metadata: Metadata,
    pub manifest: Vec<Item>,
    /// Manifest ids, in reading order.
    pub spine: Vec<String>,
    /// The id of the NCX, from the spine's `toc` attribute. EPUB 2's table of contents.
    pub ncx: Option<String>,
}

impl Package {
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.manifest.iter().find(|i| i.id == id)
    }

    /// The EPUB 3 table of contents, which announces itself in the manifest.
    pub fn nav(&self) -> Option<&Item> {
        self.manifest.iter().find(|i| i.properties.split_whitespace().any(|p| p == "nav"))
    }

    pub fn cover(&self) -> Option<&Item> {
        self.manifest.iter().find(|i| i.properties.split_whitespace().any(|p| p == "cover-image"))
    }

    /// Spine documents, as archive paths.
    pub fn documents(&self) -> Vec<&Item> {
        self.spine.iter().filter_map(|id| self.item(id)).collect()
    }
}

/// Where the package document is, per `META-INF/container.xml`.
pub fn root_path(container: &str) -> Result<String, Error> {
    let mut reader = Reader::from_str(container);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local(e.name().as_ref()) == b"rootfile" => {
                if let Some(path) = attr(&e, "full-path") {
                    return Ok(path);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Xml(e.to_string())),
            _ => {}
        }
    }
    Err(Error::Malformed("container.xml names no rootfile".into()))
}

pub fn parse(xml: &str, opf_path: &str) -> Result<Package, Error> {
    let mut package = Package::default();
    let mut reader = Reader::from_str(xml);
    let mut unknown = 0;

    // Which element's text is being collected, and for whom. An EPUB 3 `file-as` arrives as a
    // `<meta refines="#author_0">` *after* the creator it describes, so authors are indexed by
    // their id until the document ends.
    let mut collecting: Option<Collect> = None;
    let mut buffer = String::new();
    let mut ids: Vec<(String, usize)> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local(e.name().as_ref()).to_vec();
                buffer.clear();
                collecting = match name.as_slice() {
                    b"title" => Some(Collect::Title),
                    b"language" => Some(Collect::Language),
                    b"date" => Some(Collect::Date),
                    b"identifier" => Some(Collect::Identifier),
                    b"subject" => Some(Collect::Subject),
                    b"creator" => {
                        // EPUB 2 puts the filing name on the element itself; EPUB 3 refines it
                        // later. Both are read, and the later one wins by overwriting.
                        let author =
                            Author { name: String::new(), sort: attr(&e, "file-as") };
                        package.metadata.authors.push(author);
                        let index = package.metadata.authors.len() - 1;
                        if let Some(id) = attr(&e, "id") {
                            ids.push((id, index));
                        }
                        Some(Collect::Creator(index))
                    }
                    b"meta" => match (attr(&e, "property"), attr(&e, "refines")) {
                        (Some(p), Some(r)) if p == "file-as" => {
                            let target = r.trim_start_matches('#').to_owned();
                            ids.iter()
                                .find(|(id, _)| *id == target)
                                .map(|(_, i)| Collect::FileAs(*i))
                        }
                        _ => None,
                    },
                    b"spine" => {
                        package.ncx = attr(&e, "toc");
                        None
                    }
                    _ => None,
                };
            }
            Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                b"item" => {
                    if let (Some(id), Some(href)) = (attr(&e, "id"), attr(&e, "href")) {
                        package.manifest.push(Item {
                            id,
                            path: crate::archive::resolve(opf_path, &href),
                            media_type: attr(&e, "media-type").unwrap_or_default(),
                            properties: attr(&e, "properties").unwrap_or_default(),
                        });
                    }
                }
                b"itemref" => {
                    if let Some(idref) = attr(&e, "idref") {
                        package.spine.push(idref);
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if collecting.is_some() {
                    let raw = t.xml_content().unwrap_or_default();
                    buffer.push_str(&unescape(&raw, &mut unknown));
                }
            }
            Ok(Event::End(_)) => {
                let text = buffer.trim().to_owned();
                match collecting.take() {
                    Some(Collect::Title) if package.metadata.title.is_empty() => {
                        package.metadata.title = text;
                    }
                    Some(Collect::Language) => package.metadata.language = Some(text),
                    Some(Collect::Date) if package.metadata.date.is_none() => {
                        package.metadata.date = Some(text);
                    }
                    Some(Collect::Identifier) if package.metadata.identifier.is_none() => {
                        package.metadata.identifier = Some(text);
                    }
                    Some(Collect::Subject) => package.metadata.subjects.push(text),
                    Some(Collect::Creator(i)) => package.metadata.authors[i].name = text,
                    Some(Collect::FileAs(i)) => package.metadata.authors[i].sort = Some(text),
                    _ => {}
                }
                buffer.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Xml(e.to_string())),
            _ => {}
        }
    }

    // Authors with no name are an artefact of a `<creator>` that held only markup.
    package.metadata.authors.retain(|a| !a.name.is_empty());
    if package.spine.is_empty() {
        return Err(Error::Malformed("the package document has an empty spine".into()));
    }
    Ok(package)
}

enum Collect {
    Title,
    Language,
    Date,
    Identifier,
    Subject,
    Creator(usize),
    FileAs(usize),
}

#[cfg(test)]
mod tests {
    use super::parse;

    const EPUB3: &str = r##"<?xml version="1.0"?>
      <package xmlns="http://www.idpf.org/2007/opf" version="3.0">
        <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
          <dc:title>The Strange Case of Dr. Jekyll and Mr. Hyde</dc:title>
          <dc:creator id="author_0">Robert Louis Stevenson</dc:creator>
          <meta property="file-as" refines="#author_0">Stevenson, Robert Louis</meta>
          <dc:language>en</dc:language>
          <dc:date>2008-06-27</dc:date>
        </metadata>
        <manifest>
          <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
          <item id="c1" href="text/c1.xhtml" media-type="application/xhtml+xml"/>
          <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
        </manifest>
        <spine toc="ncx"><itemref idref="c1"/></spine>
      </package>"##;

    #[test]
    fn epub3_refines_the_filing_name_after_the_creator_it_describes() {
        let package = parse(EPUB3, "OEBPS/content.opf").unwrap();
        let author = &package.metadata.authors[0];
        assert_eq!(author.name, "Robert Louis Stevenson");
        assert_eq!(author.sort_key(), "Stevenson, Robert Louis");
    }

    #[test]
    fn hrefs_are_resolved_against_the_package_document() {
        let package = parse(EPUB3, "OEBPS/content.opf").unwrap();
        assert_eq!(package.item("c1").unwrap().path, "OEBPS/text/c1.xhtml");
        assert_eq!(package.nav().unwrap().path, "OEBPS/nav.xhtml");
        assert_eq!(package.ncx.as_deref(), Some("ncx"));
    }

    const FILE_AS: &str =
        r##"<meta property="file-as" refines="#author_0">Stevenson, Robert Louis</meta>"##;

    #[test]
    fn epub2_writes_the_filing_name_on_the_creator_itself() {
        let xml = EPUB3
            .replace(r##"<dc:creator id="author_0">"##, r##"<dc:creator opf:file-as="Twain, Mark">"##)
            .replace(FILE_AS, "");
        let package = parse(&xml, "OEBPS/content.opf").unwrap();
        assert_eq!(package.metadata.authors[0].sort_key(), "Twain, Mark");
    }

    #[test]
    fn a_package_with_no_spine_is_not_a_book() {
        let xml = EPUB3.replace(r##"<itemref idref="c1"/>"##, "");
        assert!(parse(&xml, "OEBPS/content.opf").is_err());
    }
}
