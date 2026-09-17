//! The three things every parser here needs from `quick-xml`.

use quick_xml::events::BytesStart;

/// The part of a qualified name after the prefix.
///
/// Namespaces are matched by local name throughout this crate. An epub in the wild binds the
/// same namespace to `opf:`, `pkg:` or no prefix at all, and a parser that matched the prefix
/// would be matching the file's typography.
pub fn local(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|b| *b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// An attribute by local name, decoded.
pub fn attr(e: &BytesStart<'_>, want: &str) -> Option<String> {
    e.attributes().flatten().find(|a| local(a.key.as_ref()) == want.as_bytes()).map(|a| {
        a.unescape_value()
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned())
    })
}

/// Named character references that are legal in XHTML and undefined in XML.
///
/// XML defines five. XHTML's DTD defines two hundred and fifty, and a document that uses one
/// without declaring the DTD is ill-formed by the letter of the spec and commonplace in a real
/// book. Resolving the ones that actually appear is cheaper than refusing the book, and
/// anything not here survives as its own source text rather than vanishing.
const ENTITIES: &[(&str, &str)] = &[
    ("nbsp", "\u{a0}"),
    ("mdash", "—"),
    ("ndash", "–"),
    ("hellip", "…"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("copy", "©"),
    ("deg", "°"),
    ("pound", "£"),
    ("sect", "§"),
    ("dagger", "†"),
    ("Dagger", "‡"),
    ("oelig", "œ"),
    ("aelig", "æ"),
    ("eacute", "é"),
    ("egrave", "è"),
    ("agrave", "à"),
    ("ccedil", "ç"),
    ("uuml", "ü"),
    ("ouml", "ö"),
    ("auml", "ä"),
    ("thinsp", "\u{2009}"),
    ("ensp", "\u{2002}"),
    ("emsp", "\u{2003}"),
    ("shy", "\u{ad}"),
];

/// Unescape text, counting what could not be resolved rather than failing on it.
pub fn unescape(raw: &str, unknown: &mut usize) -> String {
    match quick_xml::escape::unescape_with(raw, |name| {
        ENTITIES.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }) {
        Ok(text) => text.into_owned(),
        Err(_) => {
            *unknown += 1;
            raw.to_owned()
        }
    }
}
