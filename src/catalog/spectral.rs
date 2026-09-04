use std::fmt;

/// Harvard spectral class (temperature sequence, hot → cool).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SpectralClass {
    O,
    B,
    A,
    F,
    G,
    K,
    M,
    /// Wolf-Rayet nitrogen sequence
    WN,
    /// Wolf-Rayet carbon sequence
    WC,
    /// Carbon star
    C,
    /// Zirconium/carbon star (S-type)
    S,
    /// White dwarf (DA, DB, DC, DO, DQ, DZ, etc.)
    D,
    /// Could not be determined
    #[default]
    Unknown,
}

/// Yerkes luminosity class (size/brightness).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum LuminosityClass {
    /// Hypergiants / most luminous supergiants
    Ia0,
    Ia,
    Iab,
    Ib,
    /// Bright giants
    II,
    /// Giants
    III,
    /// Subgiants
    IV,
    /// Main sequence (dwarfs)
    #[default]
    V,
    /// Subdwarfs
    VI,
    /// White dwarfs
    VII,
}

/// Prefix indicating special dwarf/subdwarf classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpectralPrefix {
    /// Subdwarf (sd)
    Subdwarf,
    /// Dwarf (d) — older notation
    Dwarf,
}

bitflags::bitflags! {
    /// Spectral suffix flags indicating peculiarities.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct SpectralSuffixes: u16 {
        const EMISSION    = 0b0000_0000_0001; // e — emission lines
        const PECULIAR    = 0b0000_0000_0010; // p — peculiar
        const BROAD_LINES = 0b0000_0000_0100; // n — nebulous/broad lines
        const SHARP_LINES = 0b0000_0000_1000; // s — sharp lines
        const METALLIC    = 0b0000_0001_0000; // m — metallic
        const WEAK_LINES  = 0b0000_0010_0000; // w — weak lines
        const VARIABLE    = 0b0000_0100_0000; // var — variable
        const COMPOSITE   = 0b0000_1000_0000; // comp — composite spectrum
        const BINARY      = 0b0001_0000_0000; // SB — spectroscopic binary
        const UNCERTAIN   = 0b0010_0000_0000; // : — uncertain classification
    }
}

/// A parsed MK (Morgan-Kenaen) spectral classification.
///
/// Parses strings like "G2V", "B8Ia", "K5III", "M3.5Ve", "sdF0", "DA2", "WN5".
/// For composite spectra ("A0V + G5III"), only the primary component is parsed.
#[derive(Clone, Debug, Default)]
pub struct SpectralType {
    pub prefix: Option<SpectralPrefix>,
    pub class: SpectralClass,
    /// Numeric subtype (0.0–9.9), or `None` if not specified.
    pub subtype: Option<f32>,
    pub luminosity: Option<LuminosityClass>,
    pub suffixes: SpectralSuffixes,
}

impl SpectralType {
    /// Best-effort parse of an MK spectral type string.
    /// Returns `None` only if the string is empty or completely unrecognizable.
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }

        // For composite spectra, only parse the primary (before '+' or ' + ')
        let primary = input.split('+').next().unwrap_or(input).trim();
        // Strip parenthetical wrapping like "(G3w)F7" → take after ')'
        let primary = if let Some(idx) = primary.find(')') {
            &primary[idx + 1..]
        } else {
            primary
        };

        let mut cursor = primary;
        let mut result = SpectralType::default();

        // Parse prefix
        if cursor.starts_with("sd") {
            result.prefix = Some(SpectralPrefix::Subdwarf);
            cursor = &cursor[2..];
        } else if cursor.starts_with('d') && cursor.len() > 1 && cursor.as_bytes()[1].is_ascii_uppercase() {
            result.prefix = Some(SpectralPrefix::Dwarf);
            cursor = &cursor[1..];
        }

        // Parse spectral class
        let (class, advance) = parse_class(cursor)?;
        result.class = class;
        cursor = &cursor[advance..];

        // Parse numeric subtype
        if let Some((subtype, advance)) = parse_subtype(cursor) {
            result.subtype = Some(subtype);
            cursor = &cursor[advance..];
        }

        // Skip optional '/' alternate subtype (e.g. "A0/A1" — we already have the primary)
        if cursor.starts_with('/') {
            if let Some(end) = cursor[1..].find(|c: char| !c.is_ascii_alphanumeric() && c != '.') {
                cursor = &cursor[1 + end..];
            } else {
                cursor = "";
            }
        }

        // Skip spaces
        cursor = cursor.trim_start();

        // Parse luminosity class
        if let Some((lum, advance)) = parse_luminosity(cursor) {
            result.luminosity = Some(lum);
            cursor = &cursor[advance..];
        }

        // Parse suffixes from the remainder
        result.suffixes = parse_suffixes(cursor);

        Some(result)
    }

    /// Approximate B-V color index derived from spectral class + subtype.
    /// Useful for rendering star color. Returns a value roughly in [-0.3, 2.0].
    pub fn approx_color_index(&self) -> f32 {
        let base = match self.class {
            SpectralClass::O => -0.32,
            SpectralClass::B => -0.20,
            SpectralClass::A => 0.00,
            SpectralClass::F => 0.30,
            SpectralClass::G => 0.60,
            SpectralClass::K => 1.00,
            SpectralClass::M => 1.50,
            SpectralClass::WN | SpectralClass::WC => -0.30,
            SpectralClass::C => 1.80,
            SpectralClass::S => 1.50,
            SpectralClass::D => 0.00,
            SpectralClass::Unknown => 0.60,
        };

        // Interpolate within class using subtype (0→9 spans to next class)
        let step = match self.class {
            SpectralClass::O => 0.012,
            SpectralClass::B => 0.022,
            SpectralClass::A => 0.030,
            SpectralClass::F => 0.030,
            SpectralClass::G => 0.040,
            SpectralClass::K => 0.050,
            SpectralClass::M => 0.060,
            _ => 0.0,
        };

        base + self.subtype.unwrap_or(5.0) * step
    }
}

impl fmt::Display for SpectralType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = &self.prefix {
            match prefix {
                SpectralPrefix::Subdwarf => write!(f, "sd")?,
                SpectralPrefix::Dwarf => write!(f, "d")?,
            }
        }
        write!(f, "{:?}", self.class)?;
        if let Some(sub) = self.subtype {
            if sub.fract() == 0.0 {
                write!(f, "{}", sub as u8)?;
            } else {
                write!(f, "{sub}")?;
            }
        }
        if let Some(lum) = &self.luminosity {
            write!(f, "{lum:?}")?;
        }
        Ok(())
    }
}

fn parse_class(s: &str) -> Option<(SpectralClass, usize)> {
    // Try two-character classes first
    if s.len() >= 2 {
        match &s[..2] {
            "WN" => return Some((SpectralClass::WN, 2)),
            "WC" => return Some((SpectralClass::WC, 2)),
            // White dwarf subtypes: DA, DB, DC, DO, DQ, DZ, DX
            "DA" | "DB" | "DC" | "DO" | "DQ" | "DZ" | "DX" => {
                return Some((SpectralClass::D, 2))
            }
            _ => {}
        }
    }
    // Single-character classes
    if let Some(&ch) = s.as_bytes().first() {
        let class = match ch {
            b'O' => SpectralClass::O,
            b'B' => SpectralClass::B,
            b'A' => SpectralClass::A,
            b'F' => SpectralClass::F,
            b'G' => SpectralClass::G,
            b'K' => SpectralClass::K,
            b'M' => SpectralClass::M,
            b'C' => SpectralClass::C,
            b'S' => SpectralClass::S,
            b'W' => SpectralClass::WN, // bare 'W' treated as WN
            b'D' => SpectralClass::D,
            b'R' | b'N' => SpectralClass::C, // older carbon star notation
            _ => return None,
        };
        return Some((class, 1));
    }
    None
}

fn parse_subtype(s: &str) -> Option<(f32, usize)> {
    let mut end = 0;
    let bytes = s.as_bytes();

    // Consume digits
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == 0 {
        return None;
    }

    // Consume optional decimal point + digits
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }

    let val: f32 = s[..end].parse().ok()?;
    Some((val, end))
}

fn parse_luminosity(s: &str) -> Option<(LuminosityClass, usize)> {
    // Order matters: check longer patterns first
    let patterns: &[(&str, LuminosityClass)] = &[
        ("Ia0", LuminosityClass::Ia0),
        ("Iab", LuminosityClass::Iab),
        ("Ia", LuminosityClass::Ia),
        ("Ib", LuminosityClass::Ib),
        ("IV", LuminosityClass::IV),
        ("III", LuminosityClass::III),
        ("II", LuminosityClass::II),
        ("VII", LuminosityClass::VII),
        ("VI", LuminosityClass::VI),
        ("V", LuminosityClass::V),
        ("I", LuminosityClass::Ib), // bare 'I' → treat as Ib
    ];

    for &(pat, class) in patterns {
        if s.starts_with(pat) {
            // Make sure we're not matching a prefix of a word like "In" or "Iota"
            let after = s.as_bytes().get(pat.len());
            let valid_boundary = match after {
                None => true,
                Some(c) => !c.is_ascii_uppercase() || *c == b'I' || *c == b'V',
            };
            // Refine: after luminosity should not be another uppercase letter
            // unless it's part of luminosity notation
            if valid_boundary || !after.unwrap_or(&0).is_ascii_uppercase() {
                return Some((class, pat.len()));
            }
        }
    }
    None
}

fn parse_suffixes(s: &str) -> SpectralSuffixes {
    let mut flags = SpectralSuffixes::empty();
    let lower = s.to_ascii_lowercase();

    if lower.contains('e') && !lower.contains("fe") {
        flags |= SpectralSuffixes::EMISSION;
    }
    if lower.contains('p') && !lower.contains("comp") {
        flags |= SpectralSuffixes::PECULIAR;
    }
    if lower.contains('n') && !lower.contains("con") && !lower.contains("cn") {
        flags |= SpectralSuffixes::BROAD_LINES;
    }
    if lower.contains("var") {
        flags |= SpectralSuffixes::VARIABLE;
    }
    if lower.contains("comp") {
        flags |= SpectralSuffixes::COMPOSITE;
    }
    if lower.contains("sb") {
        flags |= SpectralSuffixes::BINARY;
    }
    if s.contains(':') {
        flags |= SpectralSuffixes::UNCERTAIN;
    }
    // 's' suffix: only if it appears as a standalone trailing letter
    if s.ends_with('s') && !s.ends_with("ss") && !lower.ends_with("var") {
        flags |= SpectralSuffixes::SHARP_LINES;
    }
    if lower.contains('m') && !lower.contains("comp") {
        flags |= SpectralSuffixes::METALLIC;
    }
    if lower.contains('w') && !lower.contains("wn") && !lower.contains("wc") {
        flags |= SpectralSuffixes::WEAK_LINES;
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let s = SpectralType::parse("G2V").unwrap();
        assert_eq!(s.class, SpectralClass::G);
        assert_eq!(s.subtype, Some(2.0));
        assert_eq!(s.luminosity, Some(LuminosityClass::V));
    }

    #[test]
    fn parse_supergiant() {
        let s = SpectralType::parse("B8Ia").unwrap();
        assert_eq!(s.class, SpectralClass::B);
        assert_eq!(s.subtype, Some(8.0));
        assert_eq!(s.luminosity, Some(LuminosityClass::Ia));
    }

    #[test]
    fn parse_decimal_subtype() {
        let s = SpectralType::parse("M3.5Ve").unwrap();
        assert_eq!(s.class, SpectralClass::M);
        assert_eq!(s.subtype, Some(3.5));
        assert_eq!(s.luminosity, Some(LuminosityClass::V));
        assert!(s.suffixes.contains(SpectralSuffixes::EMISSION));
    }

    #[test]
    fn parse_subdwarf() {
        let s = SpectralType::parse("sdF0").unwrap();
        assert_eq!(s.prefix, Some(SpectralPrefix::Subdwarf));
        assert_eq!(s.class, SpectralClass::F);
        assert_eq!(s.subtype, Some(0.0));
    }

    #[test]
    fn parse_wolf_rayet() {
        let s = SpectralType::parse("WN5").unwrap();
        assert_eq!(s.class, SpectralClass::WN);
        assert_eq!(s.subtype, Some(5.0));
    }

    #[test]
    fn parse_white_dwarf() {
        let s = SpectralType::parse("DA2").unwrap();
        assert_eq!(s.class, SpectralClass::D);
        assert_eq!(s.subtype, Some(2.0));
    }

    #[test]
    fn parse_composite() {
        let s = SpectralType::parse("A0V + G5III").unwrap();
        assert_eq!(s.class, SpectralClass::A);
        assert_eq!(s.subtype, Some(0.0));
        assert_eq!(s.luminosity, Some(LuminosityClass::V));
    }

    #[test]
    fn parse_empty() {
        assert!(SpectralType::parse("").is_none());
    }

    #[test]
    fn color_index_ordering() {
        let o = SpectralType::parse("O5V").unwrap().approx_color_index();
        let g = SpectralType::parse("G2V").unwrap().approx_color_index();
        let m = SpectralType::parse("M5III").unwrap().approx_color_index();
        assert!(o < g);
        assert!(g < m);
    }
}
