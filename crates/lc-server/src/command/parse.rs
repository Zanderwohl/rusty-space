//! The console's grammar: a command name, then words and `key:value` pairs, space-separated.
//!
//! ```text
//! teleport 0x1f2e altitude:3
//! note text:"bar \" baz" at:"12:30"
//! ```
//!
//! A value is bare or quoted. A bare one runs to the next space and may hold neither `"` nor
//! `:`; a quoted one may hold anything, with `\"` and `\\` its only escapes. Nothing ambiguous
//! is guessed at: `a:b:c` is refused with a request to quote, not split at either colon.
//!
//! Names and keys are case-insensitive and come back lowercase. Values are exactly as typed.

use lc_proto::COMMAND_LIMIT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub name: String,
    pub args: Vec<Arg>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arg {
    pub key: Option<String>,
    pub value: String,
}

/// Columns are characters from 1, as a person counts them.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("nothing to run")]
    Empty,
    #[error("the line is longer than {COMMAND_LIMIT} bytes")]
    TooLong,
    #[error("the quote at column {0} is never closed")]
    Unterminated(usize),
    #[error("\\{escape} at column {at} is not an escape: only \\\" and \\\\ are")]
    Escape { escape: char, at: usize },
    #[error("the ':' at column {0} has no value after it")]
    NoValue(usize),
    #[error("'{0}' is not a key: a key is letters, digits, '-' and '_'")]
    BadKey(String),
    #[error("the ':' at column {0} is inside a value; quote the value")]
    StrayColon(usize),
    #[error("the '\"' at column {0} is inside a word; quote the whole value")]
    StrayQuote(usize),
    #[error("put a space after the closing quote at column {0}")]
    Glued(usize),
    #[error("a command name is a bare word")]
    BadName,
}

pub fn parse(line: &str) -> Result<Parsed, ParseError> {
    if line.len() > COMMAND_LIMIT {
        return Err(ParseError::TooLong);
    }
    let chars: Vec<char> = line.chars().collect();
    let mut at = 0;
    let mut tokens = Vec::new();
    loop {
        while chars.get(at).is_some_and(|c| c.is_whitespace()) {
            at += 1;
        }
        if at == chars.len() {
            break;
        }
        tokens.push(token(&chars, &mut at)?);
    }
    let mut tokens = tokens.into_iter();
    let (first, quoted) = tokens.next().ok_or(ParseError::Empty)?;
    // Typed with the slash that opened the console, or without it.
    let name = first.value.strip_prefix('/').unwrap_or(&first.value).to_lowercase();
    if quoted || first.key.is_some() || name.is_empty() {
        return Err(ParseError::BadName);
    }
    Ok(Parsed { name, args: tokens.map(|(arg, _)| arg).collect() })
}

/// One token from `at`, which is on its first character. Leaves `at` past it.
fn token(chars: &[char], at: &mut usize) -> Result<(Arg, bool), ParseError> {
    if chars[*at] == '"' {
        let value = quoted(chars, at)?;
        return Ok((Arg { key: None, value }, true));
    }
    let start = *at;
    let word = bare(chars, at)?;
    if chars.get(*at) != Some(&':') {
        return Ok((Arg { key: None, value: word }, false));
    }
    if word.is_empty() || !word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(ParseError::BadKey(chars[start..*at].iter().collect()));
    }
    let colon = *at;
    *at += 1;
    let value = match chars.get(*at) {
        None => return Err(ParseError::NoValue(colon + 1)),
        Some(c) if c.is_whitespace() => return Err(ParseError::NoValue(colon + 1)),
        Some('"') => quoted(chars, at)?,
        Some(_) => {
            let value = bare(chars, at)?;
            if chars.get(*at) == Some(&':') {
                return Err(ParseError::StrayColon(*at + 1));
            }
            value
        }
    };
    Ok((Arg { key: Some(word.to_lowercase()), value }, false))
}

/// A `"` in a bare word is an error, never the start of a new token.
fn bare(chars: &[char], at: &mut usize) -> Result<String, ParseError> {
    let start = *at;
    while let Some(&c) = chars.get(*at) {
        if c.is_whitespace() || c == ':' {
            break;
        }
        if c == '"' {
            return Err(ParseError::StrayQuote(*at + 1));
        }
        *at += 1;
    }
    Ok(chars[start..*at].iter().collect())
}

/// A quoted value, with `at` on its opening quote. Leaves `at` past the closing one, which must
/// end the token.
fn quoted(chars: &[char], at: &mut usize) -> Result<String, ParseError> {
    let open = *at;
    *at += 1;
    let mut value = String::new();
    loop {
        match chars.get(*at) {
            None => return Err(ParseError::Unterminated(open + 1)),
            Some('"') => {
                *at += 1;
                break;
            }
            Some('\\') => {
                match chars.get(*at + 1) {
                    None => return Err(ParseError::Unterminated(open + 1)),
                    Some(&c @ ('"' | '\\')) => value.push(c),
                    Some(&escape) => return Err(ParseError::Escape { escape, at: *at + 1 }),
                }
                *at += 2;
            }
            Some(&c) => {
                value.push(c);
                *at += 1;
            }
        }
    }
    match chars.get(*at) {
        Some(c) if !c.is_whitespace() => Err(ParseError::Glued(*at)),
        _ => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(line: &str) -> (String, String) {
        let parsed = parse(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        let arg = parsed.args.into_iter().next().expect("an argument");
        (arg.key.expect("a key"), arg.value)
    }

    #[test]
    fn a_value_is_bare_or_quoted_and_a_quoted_one_holds_anything() {
        assert_eq!(keyed("x foo:bar"), ("foo".into(), "bar".into()));
        assert_eq!(keyed("x foo:\"bar baz\""), ("foo".into(), "bar baz".into()));
        assert_eq!(keyed(r#"x foo:"bar \" baz""#), ("foo".into(), "bar \" baz".into()));
        assert_eq!(keyed("x foo:\"bar:baz\""), ("foo".into(), "bar:baz".into()));
        assert_eq!(keyed(r#"x foo:"a\\b""#), ("foo".into(), "a\\b".into()));
        assert_eq!(keyed("x foo:\"\""), ("foo".into(), String::new()));
    }

    #[test]
    fn positionals_and_pairs_mix_in_the_order_typed() {
        let parsed = parse("/Teleport  12 \"two words\" Altitude:3 ").unwrap();
        assert_eq!(parsed.name, "teleport");
        let args: Vec<(Option<&str>, &str)> =
            parsed.args.iter().map(|a| (a.key.as_deref(), a.value.as_str())).collect();
        assert_eq!(args, [(None, "12"), (None, "two words"), (Some("altitude"), "3")]);
    }

    #[test]
    fn what_is_ambiguous_is_refused_and_says_where() {
        use ParseError::*;
        let cases: &[(&str, ParseError)] = &[
            ("", Empty),
            ("   ", Empty),
            ("x foo:bar:baz", StrayColon(10)),
            ("x foo:", NoValue(6)),
            ("x foo: bar", NoValue(6)),
            ("x :bar", BadKey(String::new())),
            ("x fo.o:bar", BadKey("fo.o".into())),
            ("x foo:\"bar", Unterminated(7)),
            ("x foo:\"bar\\", Unterminated(7)),
            ("x \"bar\"baz", Glued(7)),
            ("x foo:\"bar\"baz", Glued(11)),
            ("x ba\"r", StrayQuote(5)),
            (r#"x foo:"a\nb""#, Escape { escape: 'n', at: 9 }),
            ("\"x\" y", BadName),
            ("a:b c", BadName),
            ("/", BadName),
        ];
        for (line, want) in cases {
            assert_eq!(parse(line).as_ref(), Err(want), "{line:?}");
        }
    }

    #[test]
    fn a_line_past_the_limit_is_not_read() {
        let line = format!("x {}", "a".repeat(COMMAND_LIMIT));
        assert_eq!(parse(&line), Err(ParseError::TooLong));
    }

    /// Columns count characters, which is what the console shows, and not bytes.
    #[test]
    fn a_column_is_a_character_not_a_byte() {
        assert_eq!(parse("x é:a:b"), Err(ParseError::BadKey("é".into())));
        assert_eq!(parse("x ééé \"a"), Err(ParseError::Unterminated(7)));
    }
}
