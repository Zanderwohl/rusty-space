//! What is in a directory of epubs.
//!
//!     cargo run -p lc-books --example shelf -- target/library
//!     cargo run -p lc-books --example shelf -- target/library --toml
//!     cargo run -p lc-books --example shelf -- target/library/pg43-images-3.epub --toc
//!     cargo run -p lc-books --example shelf -- target/library/pg43-images-3.epub --read 2
//!     cargo run -p lc-books --example shelf -- target/library/pg43-images-3.epub --page 4
//!     cargo run -p lc-books --example shelf -- target/library --verify

use std::path::{Path, PathBuf};

use lc_books::paginate::{self, Frame};
use lc_books::{Block, Epub, Grid, LOCATION_CHARS, Page};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let after = |name: &str| {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    };
    let target = PathBuf::from(
        args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or("target/library".into()),
    );

    let files = if target.is_dir() { epubs_in(&target) } else { vec![target.clone()] };
    if files.is_empty() {
        eprintln!("no epubs in {}", target.display());
        std::process::exit(1);
    }

    if flag("--toc") {
        for file in &files {
            toc(file);
        }
    } else if flag("--read") {
        let spine: usize = after("--read").and_then(|s| s.parse().ok()).unwrap_or(0);
        for file in &files {
            read(file, spine);
        }
    } else if flag("--page") {
        let at: usize = after("--page").and_then(|s| s.parse().ok()).unwrap_or(0);
        let spine: usize = after("--spine").and_then(|s| s.parse().ok()).unwrap_or(1);
        let columns: usize = after("--columns").and_then(|s| s.parse().ok()).unwrap_or(66);
        let lines: usize = after("--lines").and_then(|s| s.parse().ok()).unwrap_or(24);
        for file in &files {
            turn(file, spine, at, Grid::frame(columns, lines));
        }
    } else if flag("--verify") {
        for file in &files {
            verify(file);
        }
    } else if flag("--toml") {
        catalogue(&files);
    } else {
        report(&files);
    }
}

fn epubs_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "epub"))
        .collect();
    files.sort();
    files
}

fn open(file: &Path) -> Option<Epub> {
    match std::fs::read(file).map_err(|e| e.to_string()).and_then(|b| {
        Epub::open(b).map_err(|e| e.to_string())
    }) {
        Ok(epub) => Some(epub),
        Err(why) => {
            println!("{}\n    FAILED: {why}\n", name(file));
            None
        }
    }
}

fn name(file: &Path) -> String {
    file.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn report(files: &[PathBuf]) {
    for file in files {
        let Some(mut epub) = open(file) else { continue };
        let authors: Vec<String> = epub
            .authors()
            .iter()
            .map(|a| format!("{} [{}]", a.name, a.sort_key()))
            .collect();

        let spine = epub.spine().len();
        let (mut chars, mut blocks, mut images, mut unknown) = (0, 0, 0, 0);
        let mut failed = 0;
        for i in 0..spine {
            match epub.document(i) {
                Ok(doc) => {
                    chars += doc.chars;
                    blocks += doc.blocks.len();
                    images += doc.images;
                    unknown += doc.unknown_entities;
                }
                Err(_) => failed += 1,
            }
        }

        println!("{}", name(file));
        println!("    title      {}", epub.title());
        println!("    authors    {}", or_none(&authors.join("; ")));
        println!(
            "    metadata   language {}, date {}, id {}",
            or_none(epub.metadata().language.as_deref().unwrap_or("")),
            or_none(epub.metadata().date.as_deref().unwrap_or("")),
            or_none(epub.metadata().identifier.as_deref().unwrap_or("")),
        );
        println!(
            "    contents   {spine} documents, {} chapters in the contents, {} cover",
            epub.toc().len(),
            if epub.cover().is_some() { "a" } else { "no" },
        );
        println!(
            "    text       {chars} characters, {} locations, {blocks} blocks, {images} images",
            chars.div_ceil(LOCATION_CHARS),
        );
        if unknown > 0 || failed > 0 {
            println!("    trouble    {unknown} unresolved entities, {failed} unreadable documents");
        }
        println!();
    }
}

fn or_none(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}

fn toc(file: &Path) {
    let Some(mut epub) = open(file) else { return };
    println!("{}  —  {}", name(file), epub.title());
    for entry in epub.toc().to_vec() {
        let indent = "  ".repeat(entry.depth as usize + 1);
        let landing = match epub.locate(&entry) {
            Some(at) => format!("spine {}, char {}", at.spine, at.char_offset),
            None => "not in the spine".into(),
        };
        println!("{indent}{}  ({landing})", entry.label);
    }
    println!();
}

fn read(file: &Path, spine: usize) {
    let Some(mut epub) = open(file) else { return };
    let doc = match epub.document(spine) {
        Ok(doc) => doc,
        Err(why) => {
            println!("{}: {why}", name(file));
            return;
        }
    };
    println!("{}  —  spine {spine}, {} characters\n", name(file), doc.chars);
    for located in doc.blocks.iter().take(40) {
        let at = located.offset;
        match &located.block {
            Block::Heading { level, text } => println!("[{at:>6}] h{level}  {}", text.plain()),
            Block::Paragraph(text) => println!("[{at:>6}]      {}", wrap(&text.plain())),
            Block::Quote(text) => println!("[{at:>6}]   >  {}", wrap(&text.plain())),
            Block::Item { depth, text, .. } => {
                println!("[{at:>6}] {}-  {}", "  ".repeat(*depth as usize), wrap(&text.plain()));
            }
            Block::Image { path, .. } => println!("[{at:>6}] img  {path}"),
            Block::Rule => println!("[{at:>6}] ---"),
        }
    }
    println!();
}

/// Wrapped to a readable measure, indented under the offset column.
fn wrap(text: &str) -> String {
    let mut out = String::new();
    let mut column = 0;
    for word in text.split_whitespace() {
        if column + word.len() > 78 {
            out.push_str("\n             ");
            column = 0;
        }
        out.push_str(word);
        out.push(' ');
        column += word.len() + 1;
    }
    out.trim_end().to_owned()
}

/// A first draft of the shelf's catalogue, for a person to correct.
fn catalogue(files: &[PathBuf]) {
    println!("# Generated by `cargo run -p lc-books --example shelf -- --toml`.");
    println!("# Every `year` below is a guess and most of them are wrong: a book states the date");
    println!("# its transcription was posted, not the year it was written. Fix them by hand.\n");
    for file in files {
        let Some(epub) = open(file) else { continue };
        println!("[[book]]");
        println!("id      = \"{}\"", slug(epub.title()));
        println!("title   = \"{}\"", escape(epub.title()));
        let authors: Vec<String> = epub
            .authors()
            .iter()
            .map(|a| {
                format!("{{ name = \"{}\", sort = \"{}\" }}", escape(&a.name), escape(a.sort_key()))
            })
            .collect();
        println!("authors = [{}]", authors.join(", "));
        match epub.metadata().date.as_deref().and_then(year) {
            Some(y) => println!("year    = {y}  # check: this is the file's date, not the book's"),
            None => println!("# year  = ?"),
        }
        let subjects: Vec<String> = epub
            .metadata()
            .subjects
            .iter()
            .map(|s| format!("\"{}\"", escape(s)))
            .collect();
        if !subjects.is_empty() {
            println!("subjects = [{}]", subjects.join(", "));
        }
        println!("file    = \"{}\"", escape(&name(file)));
        println!();
    }
}

fn year(date: &str) -> Option<i32> {
    date.get(..4).and_then(|y| y.parse().ok())
}

/// An id from the title, because the file name is not one.
///
/// `pg2488-images-3.epub` is a real name on the first shelf, and the id is in the URL a player
/// never sees but an operator reads in a log.
fn slug(s: &str) -> String {
    // Titles carry their subtitle after a colon, and it is not part of the name of the book.
    let s = s.split(':').next().unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// One page, laid out and printed — which is the only way to read a paginator.
fn turn(file: &Path, spine: usize, at: usize, frame: Frame) {
    let Some(mut epub) = open(file) else { return };
    let title = epub.title().to_owned();
    let doc = match epub.document(spine) {
        Ok(doc) => doc,
        Err(why) => {
            println!("{}: {why}", name(file));
            return;
        }
    };

    let pages: Vec<Page> = paginate::pages(&doc, &Grid, frame).collect();
    let at = at.min(pages.len().saturating_sub(1));
    let Some(page) = pages.get(at) else {
        println!("{title}: spine {spine} has nothing to read");
        return;
    };
    let columns = frame.width as usize;

    println!("┌{}┐", "─".repeat(columns + 2));
    println!("│ {:<columns$} │", trim(&title, columns));
    println!("├{}┤", "─".repeat(columns + 2));
    let mut drawn = 0;
    for slice in &page.slices {
        if slice.lead {
            let lead = Grid.lead(&doc.blocks[slice.block].block);
            for _ in 0..lead {
                println!("│ {:<columns$} │", "");
                drawn += 1;
            }
        }
        let lines = Grid.lines(&doc.blocks[slice.block].block, columns);
        for (_, line) in lines.iter().skip(slice.first_row).take(slice.rows) {
            println!("│ {:<columns$} │", trim(line, columns));
            drawn += 1;
        }
    }
    for _ in drawn..frame.height as usize {
        println!("│ {:<columns$} │", "");
    }
    println!("├{}┤", "─".repeat(columns + 2));
    let footer = format!(
        "page {} of {}   location {}   locator (spine {spine}, char {})",
        at + 1,
        pages.len(),
        page.start / LOCATION_CHARS + 1,
        page.start,
    );
    println!("│ {:<columns$} │", trim(&footer, columns));
    println!("└{}┘", "─".repeat(columns + 2));
}

fn trim(s: &str, columns: usize) -> String {
    if s.chars().count() <= columns {
        return s.to_owned();
    }
    s.chars().take(columns.saturating_sub(1)).chain(['…']).collect()
}

/// The pagination invariants, over whole books rather than a fixture.
///
/// The unit tests assert these against a chapter written to break them; this asserts them
/// against Twain, who did not know he was writing test data. Both are worth having: one finds
/// the bug, the other finds the case nobody thought to write down.
fn verify(file: &Path) {
    let Some(mut epub) = open(file) else { return };
    let frames = [Grid::frame(40, 12), Grid::frame(66, 24), Grid::frame(96, 44)];
    let mut pages = 0usize;
    let mut trouble: Vec<String> = Vec::new();

    for spine in 0..epub.spine().len() {
        let Ok(doc) = epub.document(spine) else { continue };
        for frame in frames {
            let mut expected: Vec<(usize, usize)> = Vec::new();
            for (i, located) in doc.blocks.iter().enumerate() {
                let rows = Grid.lines(&located.block, frame.width as usize).len();
                expected.extend((0..rows).map(|r| (i, r)));
            }

            let laid: Vec<Page> = paginate::pages(&doc, &Grid, frame).collect();
            let drawn: Vec<(usize, usize)> = laid
                .iter()
                .flat_map(|p| {
                    p.slices.iter().flat_map(|s| {
                        (s.first_row..s.first_row + s.rows).map(move |r| (s.block, r))
                    })
                })
                .collect();
            if drawn != expected {
                trouble.push(format!("spine {spine} at {}x{} does not tile", frame.width, frame.height));
            }
            pages += laid.len();

            for pair in laid.windows(2) {
                let Some(back) = paginate::page_before(&doc, &Grid, frame, pair[1].cursor()) else {
                    trouble.push(format!("spine {spine}: no page before {:?}", pair[1].cursor()));
                    continue;
                };
                if back.next != pair[1].cursor() {
                    trouble.push(format!("spine {spine}: paging back skips rows"));
                }
                if back.cursor() >= pair[1].cursor() {
                    trouble.push(format!("spine {spine}: paging back did not move backwards"));
                }
            }

            for page in &laid {
                let cursor = paginate::cursor_at(&doc, &Grid, frame, page.start);
                let reopened = paginate::page_at(&doc, &Grid, frame, cursor);
                if reopened.start > page.start || page.start > reopened.end {
                    trouble.push(format!("spine {spine}: locator {} does not come back", page.start));
                }
            }
        }
    }

    trouble.dedup();
    println!("{}", name(file));
    println!("    {pages} pages laid out at three frames");
    if trouble.is_empty() {
        println!("    tiles, pages back, and every locator returns\n");
    } else {
        for line in trouble.iter().take(8) {
            println!("    FAILED: {line}");
        }
        println!();
    }
}
