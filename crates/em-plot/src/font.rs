//! A 5x7 bitmap font, so the raster backend can draw labels without shipping a typeface.
//!
//! Enough for axis numbers and a title. Anything wanting real typography uses a backend that
//! has a font; the core still rasterises nothing.

pub const GLYPH_W: usize = 5;
pub const GLYPH_H: usize = 7;

/// Rows top to bottom, five bits each, most significant bit leftmost.
const fn g(rows: [u8; 7]) -> [u8; 7] {
    rows
}

const BLANK: [u8; 7] = g([0; 7]);

const DIGITS: [[u8; 7]; 10] = [
    g([0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    g([0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    g([0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    g([0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110]),
    g([0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    g([0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    g([0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    g([0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    g([0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    g([0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
];

const LETTERS: [[u8; 7]; 26] = [
    g([0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]), // A
    g([0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110]),
    g([0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
    g([0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100]),
    g([0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
    g([0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000]),
    g([0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111]),
    g([0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    g([0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    g([0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100]),
    g([0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001]),
    g([0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111]),
    g([0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001]),
    g([0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001]),
    g([0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    g([0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
    g([0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101]),
    g([0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
    g([0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
    g([0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    g([0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    g([0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100]),
    g([0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001]),
    g([0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001]),
    g([0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100]),
    g([0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111]), // Z
];

/// Bitmap for a character, or `None` for one this font does not carry.
pub fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        ' ' => BLANK,
        '0'..='9' => DIGITS[c as usize - '0' as usize],
        'A'..='Z' => LETTERS[c as usize - 'A' as usize],
        'a'..='z' => LETTERS[c as usize - 'a' as usize],
        '.' => g([0, 0, 0, 0, 0, 0b01100, 0b01100]),
        ',' => g([0, 0, 0, 0, 0b01100, 0b01100, 0b01000]),
        '-' => g([0, 0, 0, 0b11111, 0, 0, 0]),
        '+' => g([0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0]),
        ':' => g([0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0]),
        '/' => g([0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000]),
        '(' => g([0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010]),
        ')' => g([0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000]),
        '%' => g([0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011]),
        '^' => g([0b00100, 0b01010, 0b10001, 0, 0, 0, 0]),
        '_' => g([0, 0, 0, 0, 0, 0, 0b11111]),
        '=' => g([0, 0, 0b11111, 0, 0b11111, 0, 0]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_fits_its_cell() {
        for c in "0123456789ABCXYZabcxyz.,-+:/()%^_= ".chars() {
            let gl = glyph(c).unwrap_or_else(|| panic!("no glyph for {c:?}"));
            for row in gl {
                assert!(row < (1 << GLYPH_W), "{c:?} overflows {GLYPH_W} columns");
            }
        }
    }

    #[test]
    fn lowercase_falls_back_to_uppercase() {
        assert_eq!(glyph('a'), glyph('A'));
        assert_eq!(glyph('z'), glyph('Z'));
    }

    #[test]
    fn a_space_is_blank_and_a_digit_is_not() {
        assert_eq!(glyph(' '), Some(BLANK));
        assert!(glyph('8').unwrap().iter().any(|r| *r != 0));
        assert!(glyph('\u{2603}').is_none(), "unknown characters are reported, not guessed");
    }
}
