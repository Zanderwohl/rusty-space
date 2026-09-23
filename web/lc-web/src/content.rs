//! Markdown on disk, parsed once at startup.
//!
//! A post is published by a redeploy. That is accepted deliberately: it buys atomic rollback
//! of prose and code together, and it means a request is an index lookup and a template
//! render rather than a filesystem walk.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, anyhow, bail};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::Deserialize;
use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use time::Date;
use time::macros::format_description;

/// Prefix on every syntax-highlighting class, so `_content.scss` styles a namespace rather
/// than guessing which bare word came from a code block.
const SYNTAX_PREFIX: &str = "syn-";

const DATE: &[time::format_description::BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day]");

/// Roughly a slow reader's pace. The number is decoration, so precision would be a lie.
const WORDS_PER_MINUTE: usize = 220;

/// What a post writes for an image, and what it has to become.
///
/// Site resources are served from an immutable `/v/<build>/` URL, and a markdown file cannot
/// know the build id. So posts write the stable path and the renderer maps it — which also
/// means a post that moves between builds needs no editing.
const AUTHORED_ASSET_PREFIX: &str = "/static/";

#[derive(Debug, Deserialize)]
struct FrontMatter {
    title: String,
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    updated: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Post {
    pub slug: String,
    pub title: String,
    pub summary: String,
    /// From the filename, not the front matter. One source, so the two cannot disagree.
    pub date: Date,
    pub updated: Option<Date>,
    pub tags: Vec<String>,
    pub draft: bool,
    pub html: String,
    pub minutes: usize,
}

impl Post {
    pub fn date_iso(&self) -> String {
        self.date.format(DATE).expect("formatting a date cannot fail")
    }

    /// "14 March 2026" — unambiguous to every reader, unlike any all-numeric order.
    pub fn date_human(&self) -> String {
        format!("{} {} {}", self.date.day(), self.date.month(), self.date.year())
    }
}

#[derive(Debug, Clone)]
pub struct Page {
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub html: String,
}

#[derive(Debug, Default)]
pub struct Content {
    /// Newest first. Every other index is a position in this.
    posts: Vec<Post>,
    by_slug: HashMap<String, usize>,
    by_tag: BTreeMap<String, Vec<usize>>,
    pages: HashMap<String, Page>,
}

impl Content {
    /// Walks `root`, renders everything, and fails the boot on anything malformed.
    ///
    /// `drafts` is whether unfinished posts are visible at all; outside production they are
    /// loaded and marked, in production they are not loaded.
    pub fn load(root: &Path, drafts: bool) -> anyhow::Result<Self> {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let mut posts = Vec::new();
        for path in markdown_files(&root.join("posts"))? {
            let post = read_post(&path, &syntaxes)
                .with_context(|| format!("reading {}", path.display()))?;
            if post.draft && !drafts {
                continue;
            }
            posts.push(post);
        }
        posts.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.slug.cmp(&b.slug)));

        let mut by_slug = HashMap::new();
        let mut by_tag: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, post) in posts.iter().enumerate() {
            // A duplicate slug means one post is unreachable. Better to refuse to start than
            // to serve whichever happened to win the insert.
            if by_slug.insert(post.slug.clone(), i).is_some() {
                bail!("two posts share the slug '{}'", post.slug);
            }
            for tag in &post.tags {
                by_tag.entry(tag.clone()).or_default().push(i);
            }
        }

        let mut pages = HashMap::new();
        for path in markdown_files(&root.join("pages"))? {
            let page = read_page(&path, &syntaxes)
                .with_context(|| format!("reading {}", path.display()))?;
            pages.insert(page.slug.clone(), page);
        }

        tracing::info!(
            posts = posts.len(),
            tags = by_tag.len(),
            pages = pages.len(),
            "content loaded"
        );
        Ok(Content { posts, by_slug, by_tag, pages })
    }

    pub fn posts(&self) -> &[Post] {
        &self.posts
    }

    pub fn post(&self, slug: &str) -> Option<&Post> {
        self.by_slug.get(slug).map(|&i| &self.posts[i])
    }

    pub fn page(&self, slug: &str) -> Option<&Page> {
        self.pages.get(slug)
    }

    pub fn tagged(&self, tag: &str) -> Option<Vec<&Post>> {
        self.by_tag.get(tag).map(|ix| ix.iter().map(|&i| &self.posts[i]).collect())
    }

    /// Tags with their post counts, alphabetical.
    pub fn tags(&self) -> impl Iterator<Item = (&str, usize)> {
        self.by_tag.iter().map(|(t, ix)| (t.as_str(), ix.len()))
    }

    pub fn latest(&self) -> Option<&Post> {
        self.posts.first()
    }
}

fn markdown_files(dir: &Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    Ok(files)
}

fn read_post(path: &Path, syntaxes: &SyntaxSet) -> anyhow::Result<Post> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("filename is not valid UTF-8"))?;
    let (date, slug) = split_dated_name(stem)?;

    let source = std::fs::read_to_string(path)?;
    let (front, body) = split_front_matter(&source)?;
    let front: FrontMatter = toml::from_str(front).context("parsing front matter")?;

    let updated = front
        .updated
        .as_deref()
        .map(|d| Date::parse(d, DATE).with_context(|| format!("parsing updated = \"{d}\"")))
        .transpose()?;

    Ok(Post {
        slug,
        title: front.title,
        summary: front.summary,
        date,
        updated,
        tags: front.tags,
        draft: front.draft,
        minutes: reading_minutes(body),
        html: render(body, syntaxes),
    })
}

fn read_page(path: &Path, syntaxes: &SyntaxSet) -> anyhow::Result<Page> {
    let slug = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("filename is not valid UTF-8"))?
        .to_owned();
    let source = std::fs::read_to_string(path)?;
    let (front, body) = split_front_matter(&source)?;
    let front: FrontMatter = toml::from_str(front).context("parsing front matter")?;
    Ok(Page { slug, title: front.title, summary: front.summary, html: render(body, syntaxes) })
}

/// `2026-03-14-light-delay` becomes the date and `light-delay`.
///
/// The date lives in the filename alone. Carrying it in the front matter as well creates two
/// sources that can disagree, and the filename is the one that also sorts a directory listing.
fn split_dated_name(stem: &str) -> anyhow::Result<(Date, String)> {
    let (date, slug) = stem
        .char_indices()
        .nth(10)
        .map(|(i, _)| stem.split_at(i))
        .ok_or_else(|| anyhow!("expected a name like YYYY-MM-DD-slug, got '{stem}'"))?;
    let date = Date::parse(date, DATE)
        .with_context(|| format!("'{stem}' does not start with a YYYY-MM-DD date"))?;
    let slug = slug.strip_prefix('-').unwrap_or(slug);
    if slug.is_empty() {
        bail!("'{stem}' has a date but no slug after it");
    }
    Ok((date, slug.to_owned()))
}

/// Splits a leading `+++` fenced TOML block from the body.
fn split_front_matter(source: &str) -> anyhow::Result<(&str, &str)> {
    let rest = source
        .strip_prefix("+++\n")
        .ok_or_else(|| anyhow!("no `+++` front matter block at the top of the file"))?;
    let end = rest.find("\n+++").ok_or_else(|| anyhow!("front matter is not closed by `+++`"))?;
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']);
    Ok((&rest[..end], body))
}

fn reading_minutes(body: &str) -> usize {
    (body.split_whitespace().count().div_ceil(WORDS_PER_MINUTE)).max(1)
}

/// Markdown to HTML, with code blocks highlighted into classes rather than inline styles.
///
/// Classes are what make the highlighting follow the theme: an inline `style="color:…"` from
/// syntect would be the same color in light mode and dark, and there is no stylesheet rule
/// that can override it.
fn render(body: &str, syntaxes: &SyntaxSet) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_SMART_PUNCTUATION);

    let mut events = Vec::new();
    let mut code: Option<(String, String)> = None;

    for event in Parser::new_ext(body, options) {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match &kind {
                    CodeBlockKind::Fenced(l) => l.split(',').next().unwrap_or("").trim().to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::Text(text) if code.is_some() => {
                code.as_mut().expect("checked by the guard").1.push_str(&text);
            }
            Event::End(TagEnd::CodeBlock) => {
                let (lang, source) = code.take().unwrap_or_default();
                events.push(Event::Html(highlight(&lang, &source, syntaxes).into()));
            }
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url: asset_href(&dest_url).into(),
                    title,
                    id,
                }));
            }
            other => events.push(other),
        }
    }

    let mut html = String::with_capacity(body.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

/// `/static/images/x.svg` becomes `/v/<build>/images/x.svg`. Anything else is left alone,
/// so an external URL or a link to another page passes through untouched.
fn asset_href(dest: &str) -> String {
    match dest.strip_prefix(AUTHORED_ASSET_PREFIX) {
        Some(rest) => crate::assets::url(rest),
        None => dest.to_owned(),
    }
}

fn highlight(lang: &str, source: &str, syntaxes: &SyntaxSet) -> String {
    let syntax = (!lang.is_empty())
        .then(|| syntaxes.find_syntax_by_token(lang))
        .flatten()
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());

    let mut generator = ClassedHTMLGenerator::new_with_class_style(
        syntax,
        syntaxes,
        ClassStyle::SpacedPrefixed { prefix: SYNTAX_PREFIX },
    );
    for line in LinesWithEndings::from(source) {
        // A failure here is a syntax definition problem, not a content problem. Losing the
        // color is survivable; losing the code is not.
        if generator.parse_html_for_line_which_includes_newline(line).is_err() {
            return format!("<pre><code>{}</code></pre>", escape(source));
        }
    }
    let class = if lang.is_empty() {
        String::new()
    } else {
        format!(" data-language=\"{}\"", escape(lang))
    };
    format!("<pre{class}><code>{}</code></pre>", generator.finalize())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_dated_filename() {
        let (date, slug) = split_dated_name("2026-03-14-light-delay").unwrap();
        assert_eq!(date.to_string(), "2026-03-14");
        assert_eq!(slug, "light-delay");
    }

    #[test]
    fn rejects_a_name_without_a_date() {
        assert!(split_dated_name("light-delay").is_err());
        assert!(split_dated_name("2026-03-14").is_err());
        assert!(split_dated_name("not-a-date-here").is_err());
    }

    #[test]
    fn splits_front_matter() {
        let (front, body) = split_front_matter("+++\ntitle = \"x\"\n+++\n\nBody.\n").unwrap();
        assert_eq!(front, "title = \"x\"");
        assert_eq!(body, "Body.\n");
    }

    #[test]
    fn rejects_missing_or_unclosed_front_matter() {
        assert!(split_front_matter("# Just a heading\n").is_err());
        assert!(split_front_matter("+++\ntitle = \"x\"\n").is_err());
    }

    #[test]
    fn code_blocks_become_classes_not_inline_styles() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let html = render("```rust\nfn main() {}\n```\n", &syntaxes);
        assert!(html.contains("syn-"), "expected prefixed classes in {html}");
        assert!(!html.contains("style=\""), "syntect emitted inline styles: {html}");
    }

    #[test]
    fn image_paths_are_versioned() {
        let versioned = asset_href("/static/images/x.svg");
        assert!(versioned.starts_with("/v/"), "{versioned}");
        assert!(versioned.ends_with("/images/x.svg"), "{versioned}");
    }

    #[test]
    fn other_hrefs_are_left_alone() {
        assert_eq!(asset_href("https://example.com/x.png"), "https://example.com/x.png");
        assert_eq!(asset_href("/blog/some-post"), "/blog/some-post");
    }

    #[test]
    fn tables_are_enabled() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let html = render("| a | b |\n|---|---|\n| 1 | 2 |\n", &syntaxes);
        assert!(html.contains("<table>"), "{html}");
    }
}
