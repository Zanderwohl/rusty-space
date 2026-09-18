//! XHTML into blocks a renderer can lay out.
//!
//! **No CSS.** A whitelist of properties is a permanent negotiation with every publisher's
//! stylesheet, and this shelf carries books whose typography is a transcription. What survives
//! is structure — paragraphs, headings, quotes, lists, images — and the inline distinctions a
//! reader would notice if they were lost.
//!
//! Every block records the character offset it begins at, counted over the text this module
//! emits. That one coordinate is the locator, the table of contents target and the search hit;
//! see `lightcone/docs/21-library.md`.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::xml::{attr, local, unescape};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub italic: bool,
    pub bold: bool,
    pub mono: bool,
    pub superscript: bool,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub text: String,
    pub style: Style,
    /// Where a link goes, unresolved: a renderer that cannot follow one still draws it.
    pub link: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Text {
    pub runs: Vec<Run>,
}

impl Text {
    pub fn plain(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    pub fn chars(&self) -> usize {
        self.runs.iter().map(|r| r.text.chars().count()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|r| r.text.trim().is_empty())
    }
}

#[derive(Clone, Debug)]
pub enum Block {
    Heading { level: u8, text: Text },
    Paragraph(Text),
    Quote(Text),
    Item { depth: u8, ordered: bool, text: Text },
    Image { path: String, alt: String },
    Rule,
}

impl Block {
    pub fn text(&self) -> Option<&Text> {
        match self {
            Block::Heading { text, .. }
            | Block::Paragraph(text)
            | Block::Quote(text)
            | Block::Item { text, .. } => Some(text),
            Block::Image { .. } | Block::Rule => None,
        }
    }
}

/// A block and where it begins.
#[derive(Clone, Debug)]
pub struct Located {
    /// Characters from the start of this document.
    pub offset: usize,
    pub block: Block,
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    pub blocks: Vec<Located>,
    /// Total characters of body text, which is what locations are counted in.
    pub chars: usize,
    /// Character references that were neither XML's five nor in the table this crate carries.
    pub unknown_entities: usize,
    pub images: usize,
    /// Element ids, and the character offset each one sits at.
    ///
    /// A table of contents points at a fragment, and a book that puts seventy-three chapters in
    /// nine documents — which one of the first six on this shelf does — is one where the
    /// fragment is the only thing distinguishing one chapter from another.
    pub anchors: Vec<(String, usize)>,
}

impl Document {
    /// Where a fragment lands, in characters from the start of this document.
    pub fn anchor(&self, fragment: &str) -> Option<usize> {
        self.anchors.iter().find(|(id, _)| id == fragment).map(|(_, at)| *at)
    }
}

pub fn parse(xhtml: &str, path: &str) -> Document {
    Walker::new(path).run(xhtml)
}

struct Walker {
    path: String,
    doc: Document,
    runs: Vec<Run>,
    style: Style,
    link: Option<String>,
    /// Open elements that change what a flushed block *is*, innermost last.
    context: Vec<Context>,
    /// Elements whose content is markup rather than prose, by nesting depth.
    skipping: usize,
    heading: Option<u8>,
    list: Vec<bool>,
}

#[derive(Clone, Copy, PartialEq)]
enum Context {
    Quote,
    Item,
}

impl Walker {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            doc: Document::default(),
            runs: Vec::new(),
            style: Style::default(),
            link: None,
            context: Vec::new(),
            skipping: 0,
            heading: None,
            list: Vec::new(),
        }
    }

    fn run(mut self, xhtml: &str) -> Document {
        let mut reader = Reader::from_str(xhtml);
        reader.config_mut().check_end_names = false;
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = local(e.name().as_ref()).to_vec();
                    self.open(&name, &e);
                }
                Ok(Event::Empty(e)) => {
                    let name = local(e.name().as_ref()).to_vec();
                    self.empty(&name, &e);
                }
                Ok(Event::End(e)) => {
                    let name = local(e.name().as_ref()).to_vec();
                    self.close(&name);
                }
                Ok(Event::Text(t)) if self.skipping == 0 => {
                    let raw = t.xml_content().unwrap_or_default();
                    let text = unescape(&raw, &mut self.doc.unknown_entities);
                    self.push(&text);
                }
                Ok(Event::Eof) => break,
                // Ill-formed markup ends the document rather than the book: what has been read
                // so far is a chapter with a tail missing, and that is better than no chapter.
                Err(_) => break,
                _ => {}
            }
        }
        self.flush();
        self.doc
    }

    fn open(&mut self, name: &[u8], e: &quick_xml::events::BytesStart<'_>) {
        if self.skipping > 0 {
            if is_skipped(name) {
                self.skipping += 1;
            }
            return;
        }
        self.anchor(name, e);
        match name {
            b"head" | b"style" | b"script" | b"title" => self.skipping = 1,
            b"p" | b"div" | b"section" | b"td" | b"tr" | b"table" | b"tbody" => self.flush(),
            b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" => {
                self.flush();
                self.heading = Some(name[1] - b'0');
            }
            b"blockquote" => {
                self.flush();
                self.context.push(Context::Quote);
            }
            b"ol" | b"ul" => {
                self.flush();
                self.list.push(name == b"ol");
            }
            b"li" => {
                self.flush();
                self.context.push(Context::Item);
            }
            b"i" | b"em" | b"cite" => self.style.italic = true,
            b"b" | b"strong" => self.style.bold = true,
            b"code" | b"tt" | b"kbd" | b"samp" => self.style.mono = true,
            b"sup" => self.style.superscript = true,
            b"pre" => {
                self.flush();
                self.style.mono = true;
            }
            b"a" => self.link = attr(e, "href").map(|h| crate::archive::resolve(&self.path, &h)),
            _ => {}
        }
    }

    fn empty(&mut self, name: &[u8], e: &quick_xml::events::BytesStart<'_>) {
        if self.skipping > 0 {
            return;
        }
        self.anchor(name, e);
        match name {
            b"br" => self.line_break(),
            b"hr" => {
                self.flush();
                self.doc.blocks.push(Located { offset: self.doc.chars, block: Block::Rule });
            }
            // `image` is the SVG spelling, and a cover page is usually exactly that: an `svg`
            // wrapping one `image`. Skipping it would lose every cover in the shelf.
            b"img" | b"image" => {
                let href = attr(e, "src").or_else(|| attr(e, "href"));
                if let Some(href) = href {
                    self.flush();
                    self.doc.blocks.push(Located {
                        offset: self.doc.chars,
                        block: Block::Image {
                            path: crate::archive::resolve(&self.path, &href),
                            alt: attr(e, "alt").unwrap_or_default(),
                        },
                    });
                    self.doc.images += 1;
                }
            }
            _ => {}
        }
    }

    fn close(&mut self, name: &[u8]) {
        if self.skipping > 0 {
            if is_skipped(name) {
                self.skipping -= 1;
            }
            return;
        }
        match name {
            b"p" | b"div" | b"section" | b"td" | b"tr" | b"table" | b"tbody" => self.flush(),
            b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" => {
                self.flush();
                self.heading = None;
            }
            b"blockquote" => {
                self.flush();
                pop(&mut self.context, Context::Quote);
            }
            b"ol" | b"ul" => {
                self.flush();
                self.list.pop();
            }
            b"li" => {
                self.flush();
                pop(&mut self.context, Context::Item);
            }
            b"i" | b"em" | b"cite" => self.style.italic = false,
            b"b" | b"strong" => self.style.bold = false,
            b"code" | b"tt" | b"kbd" | b"samp" => self.style.mono = false,
            b"sup" => self.style.superscript = false,
            b"pre" => {
                self.flush();
                self.style.mono = false;
            }
            b"a" => self.link = None,
            _ => {}
        }
    }

    /// Append text, collapsing whitespace.
    ///
    /// Collapsing is what makes an offset mean anything: the indentation of the markup is not
    /// content, and a locator counted over it would move when the file was reformatted.
    fn push(&mut self, text: &str) {
        let mut out = String::with_capacity(text.len());
        let mut space = self
            .runs
            .last()
            .map(|r| r.text.ends_with([' ', '\n']))
            .unwrap_or(true);
        for ch in text.chars() {
            if ch.is_whitespace() {
                if !space {
                    out.push(' ');
                    space = true;
                }
            } else {
                out.push(ch);
                space = false;
            }
        }
        if out.is_empty() {
            return;
        }
        match self.runs.last_mut() {
            Some(last) if last.style == self.style && last.link == self.link => {
                last.text.push_str(&out);
            }
            _ => self.runs.push(Run { text: out, style: self.style, link: self.link.clone() }),
        }
    }

    /// Record an element's id against the text position it interrupts.
    ///
    /// `name` on an anchor is the pre-HTML5 spelling of the same thing, and a transcription made
    /// in 2004 uses it.
    fn anchor(&mut self, name: &[u8], e: &quick_xml::events::BytesStart<'_>) {
        let id = attr(e, "id").or_else(|| if name == b"a" { attr(e, "name") } else { None });
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            let pending: usize = self.runs.iter().map(|r| r.text.chars().count()).sum();
            self.doc.anchors.push((id, self.doc.chars + pending));
        }
    }

    /// A `<br>`: the one whitespace in a source file that is content, and the difference
    /// between a stanza and a paragraph.
    fn line_break(&mut self) {
        match self.runs.last_mut() {
            Some(last) if last.style == self.style && last.link == self.link => {
                last.text.push('\n');
            }
            _ => self.runs.push(Run {
                text: "\n".to_owned(),
                style: self.style,
                link: self.link.clone(),
            }),
        }
    }

    fn flush(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let mut runs = std::mem::take(&mut self.runs);
        if let Some(first) = runs.first_mut() {
            let trimmed = first.text.trim_start().to_owned();
            first.text = trimmed;
        }
        if let Some(last) = runs.last_mut() {
            let trimmed = last.text.trim_end().to_owned();
            last.text = trimmed;
        }
        runs.retain(|r| !r.text.is_empty());
        let text = Text { runs };
        if text.is_empty() {
            return;
        }
        let offset = self.doc.chars;
        self.doc.chars += text.chars();
        let block = match (self.heading, self.context.last()) {
            (Some(level), _) => Block::Heading { level, text },
            (None, Some(Context::Item)) => Block::Item {
                depth: self.list.len().saturating_sub(1).min(u8::MAX as usize) as u8,
                ordered: *self.list.last().unwrap_or(&false),
                text,
            },
            (None, Some(Context::Quote)) => Block::Quote(text),
            (None, None) => Block::Paragraph(text),
        };
        self.doc.blocks.push(Located { offset, block });
    }
}

fn pop(stack: &mut Vec<Context>, what: Context) {
    if let Some(i) = stack.iter().rposition(|c| *c == what) {
        stack.remove(i);
    }
}

fn is_skipped(name: &[u8]) -> bool {
    matches!(name, b"head" | b"style" | b"script" | b"title")
}

#[cfg(test)]
mod tests {
    use super::{Block, parse};

    const CHAPTER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
        <html xmlns="http://www.w3.org/1999/xhtml">
          <head><title>Ignored</title><style>p { color: red }</style></head>
          <body>
            <h2 id="c1">Chapter
               One</h2>
            <p>A <i>rugged</i> countenance.</p>
            <div><p id="mid">Second   paragraph.</p></div>
            <blockquote><p>Quoted.</p></blockquote>
            <ul><li>First</li><li>Second</li></ul>
            <img src="plate.jpg" alt="A plate"/>
            <hr/>
          </body>
        </html>"#;

    #[test]
    fn markup_becomes_blocks_and_head_is_not_text() {
        let doc = parse(CHAPTER, "OEBPS/c1.xhtml");
        let kinds: Vec<&str> = doc
            .blocks
            .iter()
            .map(|b| match b.block {
                Block::Heading { .. } => "h",
                Block::Paragraph(_) => "p",
                Block::Quote(_) => "quote",
                Block::Item { .. } => "li",
                Block::Image { .. } => "img",
                Block::Rule => "hr",
            })
            .collect();
        assert_eq!(kinds, ["h", "p", "p", "quote", "li", "li", "img", "hr"]);
        let text = doc.blocks.iter().filter_map(|b| b.block.text()).map(|t| t.plain());
        assert!(!text.clone().any(|t| t.contains("color")), "a stylesheet is not prose");
        assert_eq!(text.clone().next().unwrap(), "Chapter One", "markup indentation is not text");
    }

    #[test]
    fn offsets_count_the_text_that_was_emitted() {
        let doc = parse(CHAPTER, "OEBPS/c1.xhtml");
        let mut running = 0;
        for located in &doc.blocks {
            assert_eq!(located.offset, running, "a block begins where the last one ended");
            running += located.block.text().map(|t| t.chars()).unwrap_or(0);
        }
        assert_eq!(doc.chars, running);
        // An image and a rule carry no text, so they share the offset of what follows them.
        assert_eq!(doc.blocks[doc.blocks.len() - 2].offset, doc.chars);
    }

    #[test]
    fn inline_style_survives_and_splits_runs() {
        let doc = parse(CHAPTER, "OEBPS/c1.xhtml");
        let Block::Paragraph(text) = &doc.blocks[1].block else { panic!("expected a paragraph") };
        assert_eq!(text.plain(), "A rugged countenance.");
        assert_eq!(text.runs.len(), 3);
        assert!(text.runs[1].style.italic);
        assert!(!text.runs[0].style.italic);
    }

    #[test]
    fn anchors_locate_a_fragment_in_the_text() {
        let doc = parse(CHAPTER, "OEBPS/c1.xhtml");
        assert_eq!(doc.anchor("c1"), Some(0));
        // The id is on the second paragraph, so it sits after the heading and the first one.
        assert_eq!(doc.anchor("mid"), Some("Chapter One".len() + "A rugged countenance.".len()));
        assert_eq!(doc.anchor("nothing-here"), None);
    }

    #[test]
    fn a_break_is_kept_where_other_whitespace_is_collapsed() {
        let doc = parse("<p>One<br/>Two   Three</p>", "c.xhtml");
        let Block::Paragraph(text) = &doc.blocks[0].block else { panic!("expected a paragraph") };
        assert_eq!(text.plain(), "One\nTwo Three");
    }
}
