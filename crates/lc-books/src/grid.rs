//! A measurer for a character grid.
//!
//! Not a toy. It is what lets the pagination be asserted from a test and printed to a terminal,
//! and a page that can be printed is a page that can be diffed — which is more than can be said
//! for one drawn into a window. The client supplies the real measurer, over egui's fonts; this
//! one says every row is one unit tall and a column is one character wide.

use crate::paginate::{Frame, Measure, Measured, Row};
use crate::text::Block;

#[derive(Clone, Copy, Debug, Default)]
pub struct Grid;

impl Grid {
    /// A frame of so many columns by so many lines.
    pub fn frame(columns: usize, lines: usize) -> Frame {
        Frame { width: columns as f32, height: lines as f32 }
    }

    /// Blank lines above a block, for a caller printing a page rather than measuring one.
    pub fn lead(&self, block: &Block) -> usize {
        self.measure(block, 1.0).lead as usize
    }

    /// The block as it would be printed: each line, and the character it begins at.
    pub fn lines(&self, block: &Block, columns: usize) -> Vec<(usize, String)> {
        let indent = match block {
            Block::Quote(_) => 4,
            Block::Item { depth, .. } => 2 + 2 * *depth as usize,
            _ => 0,
        };
        let width = columns.saturating_sub(indent).max(8);
        match block {
            Block::Image { alt, .. } => {
                let alt = if alt.is_empty() { String::new() } else { format!(": {alt}") };
                vec![(0, format!("[image{alt}]"))]
            }
            Block::Rule => vec![(0, "* * *".to_owned())],
            other => {
                let text = other.text().map(|t| t.plain()).unwrap_or_default();
                let pad = " ".repeat(indent);
                wrap(&text, width)
                    .into_iter()
                    .map(|(at, line)| (at, format!("{pad}{line}")))
                    .collect()
            }
        }
    }
}

impl Measure for Grid {
    fn measure(&self, block: &Block, width: f32) -> Measured {
        let columns = width.max(1.0) as usize;
        let rows = self
            .lines(block, columns)
            .into_iter()
            .map(|(offset, _)| Row { height: 1.0, offset })
            .collect();
        let lead = match block {
            Block::Heading { .. } => 2.0,
            Block::Item { .. } => 0.0,
            _ => 1.0,
        };
        Measured { lead, rows }
    }
}

/// Wrap on spaces, and on a space that is not there when a word is longer than the column.
fn wrap(text: &str, width: usize) -> Vec<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut rows: Vec<(usize, String)> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut last_space: Option<usize> = None;

    while i < chars.len() {
        match chars[i] {
            // The one whitespace that is content: a stanza break, kept where the rest collapsed.
            '\n' => {
                rows.push((start, chars[start..i].iter().collect()));
                i += 1;
                start = i;
                last_space = None;
                continue;
            }
            ' ' => last_space = Some(i),
            _ => {}
        }
        if i - start >= width {
            // A word wider than the column is split rather than left to overflow; the
            // alternative is a row that does not fit the frame it was measured against.
            let at = match last_space {
                Some(space) if space > start => space,
                _ => start + width,
            };
            let at = at.min(chars.len());
            rows.push((start, chars[start..at].iter().collect()));
            start = at;
            while chars.get(start) == Some(&' ') {
                start += 1;
            }
            i = start;
            last_space = None;
            continue;
        }
        i += 1;
    }
    if start < chars.len() {
        rows.push((start, chars[start..].iter().collect()));
    }
    // Never no rows: a block with nothing in it still occupies a line, and a block with no rows
    // at all is one the paginator would step over without ever drawing.
    if rows.is_empty() {
        rows.push((0, String::new()));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn rows_know_the_character_they_begin_at() {
        let rows = wrap("the quick brown fox jumps", 10);
        assert_eq!(rows[0], (0, "the quick".to_owned()));
        assert_eq!(rows[1], (10, "brown fox".to_owned()));
        assert_eq!(rows[2], (20, "jumps".to_owned()));
        for (at, line) in &rows {
            assert_eq!(&"the quick brown fox jumps".chars().nth(*at).unwrap(), &line.chars().next().unwrap());
        }
    }

    #[test]
    fn a_word_longer_than_the_column_is_split_rather_than_overflowing() {
        let rows = wrap("antidisestablishmentarianism", 10);
        assert!(rows.iter().all(|(_, line)| line.chars().count() <= 10));
        assert_eq!(rows.iter().map(|(_, l)| l.as_str()).collect::<String>(), "antidisestablishmentarianism");
    }

    #[test]
    fn a_hard_break_ends_a_row_wherever_it_falls() {
        let rows = wrap("one\ntwo three", 40);
        assert_eq!(rows[0], (0, "one".to_owned()));
        assert_eq!(rows[1], (4, "two three".to_owned()));
    }
}
