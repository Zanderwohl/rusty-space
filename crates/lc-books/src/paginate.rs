//! Breaking a document into pages, without knowing what a page looks like.
//!
//! Only the thing drawing the text can measure it, so the measurement is a trait and everything
//! here is a function over it. That is what lets the algorithm — where the bugs are — be tested
//! against a measurer whose every row is one unit tall, with no window and no font.
//!
//! **Rows, not blocks.** A paragraph routinely exceeds a whole page, so a break lands on a line;
//! and because a row knows the character it begins at, the break *is* the locator, with no
//! second derivation to disagree with the first.

use crate::text::{Block, Document};

/// The column being laid out, in whatever unit the measurer works in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub width: f32,
    pub height: f32,
}

/// A place to resume from: a block, and a row inside it.
///
/// **Not a locator.** A locator is a character offset, which is what survives a font change and
/// is what gets saved; two adjacent images share one, because neither holds any text. Paging
/// forward needs to tell them apart, so traversal is by index and only the saved position is by
/// character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    pub block: usize,
    pub row: usize,
}

/// One laid-out line.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub height: f32,
    /// Characters from the start of this block's own text.
    pub offset: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Measured {
    /// Space above this block, applied only when it is not the first thing on a page.
    pub lead: f32,
    pub rows: Vec<Row>,
}

pub trait Measure {
    fn measure(&self, block: &Block, width: f32) -> Measured;
}

/// Part of one block, on one page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    pub block: usize,
    pub first_row: usize,
    pub rows: usize,
    /// Rows the whole block has, so a renderer can tell a continuation from a whole paragraph.
    pub total: usize,
    /// Whether the space above this block is drawn. False at the top of a page.
    pub lead: bool,
}

impl Slice {
    pub fn continues(&self) -> bool {
        self.first_row + self.rows < self.total
    }

    pub fn starts_mid_block(&self) -> bool {
        self.first_row > 0
    }
}

#[derive(Clone, Debug)]
pub struct Page {
    pub slices: Vec<Slice>,
    /// Where to resume. Equal to `from` only for an empty page, which cannot happen.
    pub next: Cursor,
    /// The locator to save if the reader closes the book here.
    pub start: usize,
    /// One past the last character on the page, which is the next page's `start`.
    pub end: usize,
}

impl Page {
    pub fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    pub fn cursor(&self) -> Cursor {
        match self.slices.first() {
            Some(s) => Cursor { block: s.block, row: s.first_row },
            None => self.next,
        }
    }
}

/// The page that begins at `from`.
///
/// Laid out forward from that point and nowhere else: a page is never computed by paginating the
/// chapter and indexing into it, because a book is long and a reader only ever looks at one page.
pub fn page_at<M: Measure>(doc: &Document, measure: &M, frame: Frame, from: Cursor) -> Page {
    let mut slices: Vec<Slice> = Vec::new();
    let mut used = 0.0f32;
    let mut start = doc.chars;
    let mut block = from.block;
    let mut row = from.row;

    while block < doc.blocks.len() {
        let measured = measure.measure(&doc.blocks[block].block, frame.width);
        let total = measured.rows.len();
        if row >= total {
            block += 1;
            row = 0;
            continue;
        }
        let draw_lead = !slices.is_empty();
        let mut height = used + if draw_lead { measured.lead } else { 0.0 };
        let mut taken = 0;
        while row + taken < total {
            let next_row = measured.rows[row + taken].height;
            // Every page holds at least one row, whatever the frame. A page that could be empty
            // is a page that does not advance, and pagination that does not advance never ends.
            let forced = slices.is_empty() && taken == 0;
            if height + next_row > frame.height && !forced {
                break;
            }
            height += next_row;
            taken += 1;
        }
        if taken == 0 {
            break;
        }
        if slices.is_empty() {
            start = doc.blocks[block].offset + measured.rows[row].offset;
        }
        slices.push(Slice { block, first_row: row, rows: taken, total, lead: draw_lead });
        used = height;
        if row + taken < total {
            break;
        }
        block += 1;
        row = 0;
    }

    tidy(doc, measure, frame, &mut slices);
    let (next, end) = match slices.last() {
        Some(last) if last.continues() => {
            let measured = measure.measure(&doc.blocks[last.block].block, frame.width);
            let row = last.first_row + last.rows;
            let end = doc.blocks[last.block].offset + measured.rows[row].offset;
            (Cursor { block: last.block, row }, end)
        }
        Some(last) => {
            let located = &doc.blocks[last.block];
            let end = located.offset + located.block.text().map_or(0, |t| t.chars());
            (Cursor { block: last.block + 1, row: 0 }, end)
        }
        None => (Cursor { block: doc.blocks.len(), row: 0 }, doc.chars),
    };
    Page { slices, next, start, end }
}

/// Move a line or two down a page, where leaving them costs more than the space.
///
/// Both rules cost a fraction of a page and are refused when they would cost more than a third
/// of one, which is the difference between typesetting and a hole.
fn tidy<M: Measure>(doc: &Document, measure: &M, frame: Frame, slices: &mut Vec<Slice>) {
    let Some(&last) = slices.last() else { return };
    if slices.len() < 2 {
        return;
    }
    let measured = measure.measure(&doc.blocks[last.block].block, frame.width);
    let height = |from: usize, count: usize| -> f32 {
        measured.rows.iter().skip(from).take(count).map(|r| r.height).sum()
    };
    let budget = frame.height / 3.0;

    // A heading is the first line of what follows it, and a page that ends on one has promised
    // something it does not deliver.
    let heading = matches!(doc.blocks[last.block].block, Block::Heading { .. });
    if heading && !last.continues() && height(last.first_row, last.rows) <= budget {
        slices.pop();
        return;
    }

    if !last.continues() {
        return;
    }
    let left = last.total - (last.first_row + last.rows);
    // One line of a paragraph at the foot of a page, or one left behind at the head of the next.
    if last.rows == 1 && height(last.first_row, 1) <= budget {
        slices.pop();
    } else if left == 1 && last.rows >= 2 && height(last.first_row + last.rows - 1, 2) <= budget {
        if last.rows == 2 {
            slices.pop();
        } else {
            slices.last_mut().expect("checked above").rows -= 1;
        }
    }
}

/// Pages from the top of the document.
pub fn pages<'a, M: Measure>(doc: &'a Document, measure: &'a M, frame: Frame) -> Pages<'a, M> {
    pages_from(doc, measure, frame, Cursor::default())
}

pub fn pages_from<'a, M: Measure>(
    doc: &'a Document,
    measure: &'a M,
    frame: Frame,
    from: Cursor,
) -> Pages<'a, M> {
    Pages { doc, measure, frame, at: from }
}

pub struct Pages<'a, M> {
    doc: &'a Document,
    measure: &'a M,
    frame: Frame,
    at: Cursor,
}

impl<M: Measure> Iterator for Pages<'_, M> {
    type Item = Page;

    fn next(&mut self) -> Option<Page> {
        if self.at.block >= self.doc.blocks.len() {
            return None;
        }
        let page = page_at(self.doc, self.measure, self.frame, self.at);
        if page.is_empty() || page.next <= self.at {
            return None;
        }
        self.at = page.next;
        Some(page)
    }
}

/// The page that ends exactly where `before` begins.
///
/// Built **backwards**, a row at a time, rather than by laying pages out forward from a guessed
/// origin and keeping the last one. The forward guess is the obvious implementation and it is
/// wrong: a tiling depends on where it started, so the page it produces need not end where the
/// reader actually is, and the rows in between are skipped. Going backwards from the reader's
/// own position cannot do that — the page it returns ends where they are, by construction.
///
/// No widow and orphan pass here. The foot of this page is a break that was already tidied on
/// the way forward, and re-tidying it would move the boundary the reader just came through.
pub fn page_before<M: Measure>(
    doc: &Document,
    measure: &M,
    frame: Frame,
    before: Cursor,
) -> Option<Page> {
    let mut taken: Vec<Cursor> = Vec::new();
    let mut used = 0.0f32;
    // The block whose first row is currently the top of the page: it gets its space above only
    // once something is added over it.
    let mut opened: Option<usize> = None;
    let mut at = before;

    while let Some(previous) = step_back(doc, measure, frame, at) {
        let measured = measure.measure(&doc.blocks[previous.block].block, frame.width);
        let mut cost = measured.rows[previous.row].height;
        if let Some(below) = opened {
            cost += measure.measure(&doc.blocks[below].block, frame.width).lead;
        }
        if used + cost > frame.height && !taken.is_empty() {
            break;
        }
        used += cost;
        taken.push(previous);
        opened = (previous.row == 0).then_some(previous.block);
        at = previous;
    }

    if taken.is_empty() {
        return None;
    }

    let mut slices: Vec<Slice> = Vec::new();
    for cursor in taken.iter().rev() {
        let total = measure.measure(&doc.blocks[cursor.block].block, frame.width).rows.len();
        match slices.last_mut() {
            Some(last) if last.block == cursor.block => last.rows += 1,
            _ => slices.push(Slice {
                block: cursor.block,
                first_row: cursor.row,
                rows: 1,
                total,
                lead: !slices.is_empty(),
            }),
        }
    }

    // The foot of this page was tidied on the way forward and must not move again — it is the
    // boundary the reader just came through. The head is still free, and a single line of a
    // paragraph left at the top of a page is the same fault as one left at the bottom.
    if slices.len() > 1
        && let Some(first) = slices.first()
        && first.starts_mid_block()
        && first.rows == 1
    {
        slices.remove(0);
        if let Some(first) = slices.first_mut() {
            first.lead = false;
        }
    }

    let first = slices.first().expect("checked above");
    let measured = measure.measure(&doc.blocks[first.block].block, frame.width);
    let start = doc.blocks[first.block].offset + measured.rows[first.first_row].offset;
    let end = offset_of(doc, measure, frame, before);
    Some(Page { slices, next: before, start, end })
}

/// The row before this cursor, stepping over blocks that measure to nothing.
fn step_back<M: Measure>(
    doc: &Document,
    measure: &M,
    frame: Frame,
    at: Cursor,
) -> Option<Cursor> {
    let mut block = at.block.min(doc.blocks.len());
    let mut row = if at.block >= doc.blocks.len() { 0 } else { at.row };
    loop {
        if row > 0 {
            return Some(Cursor { block, row: row - 1 });
        }
        if block == 0 {
            return None;
        }
        block -= 1;
        row = measure.measure(&doc.blocks[block].block, frame.width).rows.len();
    }
}

/// The character a cursor sits at.
fn offset_of<M: Measure>(doc: &Document, measure: &M, frame: Frame, at: Cursor) -> usize {
    let Some(located) = doc.blocks.get(at.block) else { return doc.chars };
    let measured = measure.measure(&located.block, frame.width);
    match measured.rows.get(at.row) {
        Some(row) => located.offset + row.offset,
        None => located.offset + located.block.text().map_or(0, |t| t.chars()),
    }
}

/// The cursor for a saved locator.
///
/// The character offset is what was written down; this is where it lands in the document as it
/// is laid out now, which is the whole point of storing an offset rather than a page.
///
/// **The earliest block that could hold that character wins.** A plate and the paragraph after
/// it begin at the same offset, because a plate is not made of characters, and an offset alone
/// cannot say which of them the reader was looking at. Resolving to the earlier one shows them
/// the plate again; resolving to the later one skips it. Being shown something twice is a
/// smaller wrong than never being shown it.
pub fn cursor_at<M: Measure>(
    doc: &Document,
    measure: &M,
    frame: Frame,
    char_offset: usize,
) -> Cursor {
    let block = doc.blocks.iter().position(|b| contains(b, char_offset));
    let block = block.unwrap_or_else(|| doc.blocks.len().saturating_sub(1));
    let Some(located) = doc.blocks.get(block) else { return Cursor::default() };
    let within = char_offset.saturating_sub(located.offset);
    let measured = measure.measure(&located.block, frame.width);
    let row = measured.rows.iter().rposition(|r| r.offset <= within).unwrap_or(0);
    Cursor { block, row }
}

/// Whether a character belongs to this block, counting a block with no characters as beginning
/// at its own offset.
fn contains(located: &crate::text::Located, char_offset: usize) -> bool {
    let chars = located.block.text().map_or(0, |t| t.chars());
    if chars == 0 {
        located.offset >= char_offset
    } else {
        located.offset + chars > char_offset
    }
}

#[cfg(test)]
mod tests {
    use super::{Cursor, Frame, Measure, cursor_at, page_at, page_before, pages, pages_from};
    use crate::grid::Grid;
    use crate::text::{Block, Document, parse};

    /// A chapter with everything in it that breaks a page: a heading, paragraphs longer than a
    /// frame, one-line paragraphs, quotes, a list and a plate.
    fn chapter() -> Document {
        let mut xhtml = String::from("<body><h2>Chapter One</h2>");
        for i in 0..12 {
            let sentences = 1 + (i % 5) * 4;
            xhtml.push_str("<p>");
            for s in 0..sentences {
                xhtml.push_str(&format!("Paragraph {i} sentence {s} runs on for a while. "));
            }
            xhtml.push_str("</p>");
            if i % 4 == 3 {
                xhtml.push_str("<blockquote><p>A quoted line, and another.</p></blockquote>");
            }
            if i % 5 == 4 {
                xhtml.push_str("<ul><li>First item</li><li>Second item</li></ul>");
                xhtml.push_str("<img src=\"plate.jpg\" alt=\"A plate\"/>");
            }
        }
        xhtml.push_str("</body>");
        parse(&xhtml, "c1.xhtml")
    }

    /// Every (block, row) in the document, in order.
    fn all_rows<M: Measure>(doc: &Document, measure: &M, width: f32) -> Vec<(usize, usize)> {
        doc.blocks
            .iter()
            .enumerate()
            .flat_map(|(i, b)| {
                (0..measure.measure(&b.block, width).rows.len()).map(move |r| (i, r))
            })
            .collect()
    }

    fn laid_out<M: Measure>(doc: &Document, measure: &M, frame: Frame) -> Vec<(usize, usize)> {
        pages(doc, measure, frame)
            .flat_map(|p| {
                p.slices
                    .into_iter()
                    .flat_map(|s| (s.first_row..s.first_row + s.rows).map(move |r| (s.block, r)))
            })
            .collect()
    }

    #[test]
    fn the_pages_tile_the_document_exactly_once() {
        let doc = chapter();
        for (columns, lines) in [(40, 12), (64, 20), (80, 40), (30, 6)] {
            let frame = Grid::frame(columns, lines);
            assert_eq!(
                laid_out(&doc, &Grid, frame),
                all_rows(&doc, &Grid, frame.width),
                "no row may be dropped or drawn twice at {columns}x{lines}"
            );
        }
    }

    #[test]
    fn a_frame_too_small_for_a_single_row_still_terminates() {
        let doc = chapter();
        let frame = Frame { width: 40.0, height: 0.25 };
        let pages: Vec<_> = pages(&doc, &Grid, frame).collect();
        assert!(pages.iter().all(|p| !p.is_empty()), "every page holds at least one row");
        assert_eq!(laid_out(&doc, &Grid, frame), all_rows(&doc, &Grid, frame.width));
    }

    #[test]
    fn paging_back_skips_nothing_and_turning_forward_returns() {
        let doc = chapter();
        let frame = Grid::frame(64, 20);
        let forward: Vec<_> = pages(&doc, &Grid, frame).collect();
        assert!(forward.len() > 3, "the fixture is too short to be testing this");
        for pair in forward.windows(2) {
            let here = pair[1].cursor();
            let back = page_before(&doc, &Grid, frame, here).expect("a page before this one");
            // A page laid out backwards need not match the forward tiling — it is a different
            // page, laid out from a different end. What it must do is end where the reader is,
            // so that nothing falls between the two and the page they came from is one turn away.
            assert_eq!(back.next, here, "no rows fall between the two pages");
            assert!(back.cursor() < here, "paging back moves backwards");
        }
        assert!(page_before(&doc, &Grid, frame, forward[0].cursor()).is_none());
    }

    #[test]
    fn paging_back_through_a_chapter_reaches_the_top_without_repeating_a_row() {
        let doc = chapter();
        let frame = Grid::frame(64, 20);
        let last = pages(&doc, &Grid, frame).last().expect("a chapter has pages");
        let mut at = last.cursor();
        let mut seen: Vec<(usize, usize)> = Vec::new();
        while let Some(page) = page_before(&doc, &Grid, frame, at) {
            assert_eq!(page.next, at);
            let rows = page
                .slices
                .iter()
                .flat_map(|s| (s.first_row..s.first_row + s.rows).map(move |r| (s.block, r)));
            seen.splice(0..0, rows);
            at = page.cursor();
            assert!(seen.len() < 10_000, "paging back is not making progress");
        }
        assert_eq!(at, Cursor::default(), "paging back arrives at the top");
        let mut expected = all_rows(&doc, &Grid, frame.width);
        expected.truncate(expected.len() - last.slices.iter().map(|s| s.rows).sum::<usize>());
        assert_eq!(seen, expected, "the way back covers the book exactly once");
    }

    #[test]
    fn a_saved_offset_comes_back_to_the_same_place() {
        let doc = chapter();
        let frame = Grid::frame(64, 20);
        for page in pages(&doc, &Grid, frame) {
            let reopened = page_at(&doc, &Grid, frame, cursor_at(&doc, &Grid, frame, page.start));
            // Not necessarily the same cursor: a plate and the paragraph under it share an
            // offset, so reopening can land one block earlier. What it may never do is land
            // after the character that was written down.
            assert!(reopened.cursor() <= page.cursor());
            assert!(reopened.start <= page.start && page.start <= reopened.end);
        }
    }

    #[test]
    fn a_locator_survives_a_resize() {
        let doc = chapter();
        let narrow = Grid::frame(40, 12);
        let wide = Grid::frame(78, 30);
        for page in pages(&doc, &Grid, narrow) {
            let cursor = cursor_at(&doc, &Grid, wide, page.start);
            let reopened = page_at(&doc, &Grid, wide, cursor);
            assert!(
                reopened.start <= page.start && page.start <= reopened.end,
                "the reader stays on the sentence they were reading"
            );
        }
    }

    #[test]
    fn a_page_does_not_end_on_a_heading() {
        let doc = chapter();
        for (columns, lines) in [(40, 12), (64, 20), (80, 40)] {
            let frame = Grid::frame(columns, lines);
            for page in pages(&doc, &Grid, frame) {
                let last = page.slices.last().expect("no page is empty");
                let heading = matches!(doc.blocks[last.block].block, Block::Heading { .. });
                assert!(
                    !heading || page.slices.len() == 1,
                    "a heading alone at the foot of a page at {columns}x{lines}"
                );
            }
        }
    }

    #[test]
    fn no_paragraph_is_broken_to_leave_a_single_line_behind() {
        let doc = chapter();
        let frame = Grid::frame(64, 20);
        for page in pages(&doc, &Grid, frame) {
            let last = page.slices.last().expect("no page is empty");
            if !last.continues() || page.slices.len() == 1 {
                continue;
            }
            assert_ne!(last.rows, 1, "an orphan: one line held back at the foot");
            assert_ne!(last.total - (last.first_row + last.rows), 1, "a widow: one line carried");
        }
    }

    #[test]
    fn paging_from_the_middle_of_a_block_resumes_at_that_row() {
        let doc = chapter();
        let frame = Grid::frame(40, 8);
        let first = page_at(&doc, &Grid, frame, Cursor::default());
        let second = page_at(&doc, &Grid, frame, first.next);
        assert_eq!(second.cursor(), first.next);
        let mut from_middle = pages_from(&doc, &Grid, frame, first.next);
        assert_eq!(from_middle.next().map(|p| p.cursor()), Some(second.cursor()));
    }
}
