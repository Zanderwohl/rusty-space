//! The table of contents, from whichever of the two forms the book carries.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::xml::{attr, local, unescape};

#[derive(Clone, Debug)]
pub struct TocEntry {
    pub label: String,
    /// Archive path of the document this entry lands in.
    pub path: String,
    /// The element inside it, when the entry points at one. Not decoration: a book that puts
    /// many chapters in one document distinguishes them by nothing else.
    pub fragment: Option<String>,
    /// Nesting, from zero.
    pub depth: u8,
}

/// Split an href into the document it names and the element inside it.
fn target(base: &str, href: &str) -> (String, Option<String>) {
    let (path, fragment) = match href.split_once('#') {
        Some((path, fragment)) if !fragment.is_empty() => (path, Some(fragment.to_owned())),
        _ => (href, None),
    };
    (crate::archive::resolve(base, path), fragment)
}

/// EPUB 3: a `<nav epub:type="toc">` of nested lists.
pub fn from_nav(xhtml: &str, nav_path: &str) -> Vec<TocEntry> {
    let mut reader = Reader::from_str(xhtml);
    let mut entries = Vec::new();
    let mut unknown = 0;

    // Depth is the `ol` nesting inside the toc nav, so the first list is depth zero.
    let mut in_toc = false;
    let mut nav_depth = 0usize;
    let mut list_depth = 0usize;
    let mut pending: Option<(String, Option<String>)> = None;
    let mut label = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"nav" => {
                    nav_depth += 1;
                    let kind = attr(&e, "type").unwrap_or_default();
                    // A book with no declared type has one nav and it is the contents.
                    if kind.split_whitespace().any(|t| t == "toc") || kind.is_empty() {
                        in_toc = true;
                    }
                }
                b"ol" | b"ul" if in_toc => list_depth += 1,
                b"a" if in_toc => {
                    pending = attr(&e, "href").map(|h| target(nav_path, &h));
                    label.clear();
                }
                _ => {}
            },
            Ok(Event::Text(t)) if pending.is_some() => {
                let raw = t.xml_content().unwrap_or_default();
                label.push_str(&unescape(&raw, &mut unknown));
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"nav" => {
                    nav_depth = nav_depth.saturating_sub(1);
                    if nav_depth == 0 {
                        in_toc = false;
                    }
                }
                b"ol" | b"ul" if in_toc => list_depth = list_depth.saturating_sub(1),
                b"a" => {
                    if let Some((path, fragment)) = pending.take() {
                        let label = collapse(&label);
                        if !label.is_empty() {
                            let depth = list_depth.saturating_sub(1).min(u8::MAX as usize) as u8;
                            entries.push(TocEntry { label, path, fragment, depth });
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    entries
}

/// EPUB 2: an NCX of nested `navPoint`s.
pub fn from_ncx(xml: &str, ncx_path: &str) -> Vec<TocEntry> {
    let mut reader = Reader::from_str(xml);
    let mut entries = Vec::new();
    let mut unknown = 0;

    let mut depth = 0usize;
    let mut label = String::new();
    let mut in_label = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"navPoint" => {
                    depth += 1;
                    label.clear();
                }
                b"text" => in_label = true,
                _ => {}
            },
            // Emitted here rather than at the end of the `navPoint`, because a `navPoint` holds
            // its children: waiting for its end would emit a parent after its own sections, and
            // — having had its label overwritten by the last of them — with the wrong words.
            Ok(Event::Empty(e)) if local(e.name().as_ref()) == b"content" => {
                let label = collapse(&label);
                if let Some(src) = attr(&e, "src")
                    && !label.is_empty()
                {
                    let (path, fragment) = target(ncx_path, &src);
                    entries.push(TocEntry {
                        label,
                        path,
                        fragment,
                        depth: depth.saturating_sub(1).min(u8::MAX as usize) as u8,
                    });
                }
            }
            Ok(Event::Text(t)) if in_label => {
                let raw = t.xml_content().unwrap_or_default();
                label.push_str(&unescape(&raw, &mut unknown));
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"text" => in_label = false,
                b"navPoint" => depth = depth.saturating_sub(1),
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    entries
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
