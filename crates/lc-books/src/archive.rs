//! The container. An epub is a zip, and this is the only module that knows that.

use std::io::{Cursor, Read};

use crate::Error;

pub struct Archive {
    zip: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl Archive {
    pub fn open(bytes: Vec<u8>) -> Result<Self, Error> {
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Zip(e.to_string()))?;
        Ok(Self { zip })
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        // Tried as written first: a lookup that only ever succeeded case-insensitively would
        // hide a manifest that disagrees with its own archive.
        let name = if self.zip.index_for_name(path).is_some() {
            path.to_owned()
        } else {
            self.zip
                .file_names()
                .find(|n| n.eq_ignore_ascii_case(path))
                .map(str::to_owned)
                .ok_or_else(|| Error::Missing(path.to_owned()))?
        };
        let mut entry = self.zip.by_name(&name).map_err(|e| Error::Zip(e.to_string()))?;
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| Error::Zip(e.to_string()))?;
        Ok(bytes)
    }

    pub fn read_text(&mut self, path: &str) -> Result<String, Error> {
        let bytes = self.read(path)?;
        // Lossy: one undecodable byte in a chapter should cost that character, not the book.
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Resolve an href against the document that carried it.
///
/// Hrefs are URI references relative to their own document, so they are percent-encoded and may
/// climb out of its directory; zip entry names are neither. This is the conversion between them,
/// and every path that reaches [`Archive::read`] has been through it.
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.split(['#', '?']).next().unwrap_or("");
    let href = percent_decode(href);
    if href.starts_with('/') {
        return href.trim_start_matches('/').to_owned();
    }
    let mut parts: Vec<&str> = Vec::new();
    let dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    for segment in dir.split('/').chain(href.split('/')) {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn hrefs_resolve_against_their_own_document() {
        assert_eq!(resolve("OEBPS/content.opf", "chapter1.xhtml"), "OEBPS/chapter1.xhtml");
        assert_eq!(resolve("OEBPS/text/c1.xhtml", "../images/a.jpg"), "OEBPS/images/a.jpg");
        assert_eq!(resolve("OEBPS/c1.xhtml", "c2.xhtml#part2"), "OEBPS/c2.xhtml");
        assert_eq!(resolve("OEBPS/c1.xhtml", "A%20Title.xhtml"), "OEBPS/A Title.xhtml");
        assert_eq!(resolve("content.opf", "OEBPS/c1.xhtml"), "OEBPS/c1.xhtml");
    }
}
