//! What a command takes, and binding a [`Parsed`] line to it for one asker.
//!
//! **Below its level a thing does not exist.** A command the asker may not run is answered as
//! a name nobody has, an argument they may not supply as one the command does not take, and
//! `help` lists neither. A console that said "not permitted" would tell a player which
//! administrative commands a shard has, and what they take.
//!
//! An argument is named, and may be given as `name:value` or positionally: a positional fills the
//! first argument not yet given, in the order declared. Each is required, optional, or has a
//! default that is read exactly as if it had been typed.

use std::fmt::Write as _;

use super::parse::Parsed;
use crate::ability::Level;

/// One command, as `help` shows it and as [`bind`] reads it.
pub struct Spec {
    pub name: &'static str,
    pub verb: Verb,
    /// The least senior level that may run it.
    pub level: Level,
    /// One line, for the list and under the usage line.
    pub summary: &'static str,
    pub args: &'static [ArgSpec],
}

/// What a command does. Matched exhaustively where commands run, so a row in the table with no
/// code behind it is a compile error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Help,
    Teleport,
    Where,
    WhoIs,
    Energize,
    Drain,
    Chart,
    RefitMagic,
    RefitFinish,
    Stage,
}

pub struct ArgSpec {
    pub name: &'static str,
    pub kind: Kind,
    pub need: Need,
    /// The least senior level that may supply it.
    pub level: Level,
    pub help: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub enum Need {
    Required,
    /// Absent unless given.
    Optional,
    /// This, as if typed.
    Default(&'static str),
}

pub enum Kind {
    /// One of these, case-insensitively. Not a prefix of one.
    Word(&'static [&'static str]),
    /// A number, inside the widest of these ranges the asker's level reaches.
    Number(&'static [Limit]),
    /// A catalog or body identifier, as `where` prints them: decimal, or hexadecimal with
    /// `0x`.
    Id,
    /// A whole number from zero to this.
    Count(u32),
    /// Anything at all.
    Text,
}

/// How far a [`Kind::Number`] may go for everyone at `level` or more senior.
pub struct Limit {
    pub level: Level,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Word(&'static str),
    Number(f64),
    Id(u64),
    Count(u32),
    Text(String),
}

/// A command's arguments, by name, after binding. An optional argument that was not given is
/// absent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bound {
    values: Vec<(&'static str, Value)>,
}

impl Bound {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    pub fn word(&self, name: &str) -> Option<&'static str> {
        match self.get(name)? {
            Value::Word(word) => Some(word),
            _ => None,
        }
    }

    pub fn number(&self, name: &str) -> Option<f64> {
        match self.get(name)? {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn id(&self, name: &str) -> Option<u64> {
        match self.get(name)? {
            Value::Id(id) => Some(*id),
            _ => None,
        }
    }

    pub fn count(&self, name: &str) -> Option<u32> {
        match self.get(name)? {
            Value::Count(n) => Some(*n),
            _ => None,
        }
    }

    pub fn text(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum BindError {
    #[error("{command} takes no argument '{arg}'")]
    UnknownArg { command: &'static str, arg: String },
    #[error("{0} was given twice")]
    Repeated(&'static str),
    #[error("nothing left to take '{value}'; {usage}")]
    TooMany { value: String, usage: String },
    #[error("{arg} is required; {usage}")]
    Missing { arg: &'static str, usage: String },
    #[error("{arg} is one of {choices}, not '{value}'")]
    NotOneOf { arg: &'static str, value: String, choices: String },
    #[error("{arg} is a number, not '{value}'")]
    NotANumber { arg: &'static str, value: String },
    #[error("{arg} is from {min} to {max}")]
    OutOfRange { arg: &'static str, min: f64, max: f64 },
    #[error("{arg} is a whole number, not '{value}'")]
    NotACount { arg: &'static str, value: String },
    #[error("{arg} is an identifier, decimal or 0x hexadecimal, not '{value}'")]
    NotAnId { arg: &'static str, value: String },
}

impl Spec {
    /// The arguments `level` may supply, in the order positionals fill them.
    pub fn args_for(&self, level: Level) -> impl Iterator<Item = &'static ArgSpec> + '_ {
        self.args.iter().filter(move |arg| level.at_least(arg.level))
    }

    /// `teleport <target> [altitude:2] [star:<id>]`, as `level` may type it.
    pub fn usage(&self, level: Level) -> String {
        let mut out = self.name.to_string();
        for arg in self.args_for(level) {
            let _ = match arg.need {
                Need::Required => write!(out, " <{}>", arg.name),
                Need::Optional => write!(out, " [{}:{}]", arg.name, arg.kind.placeholder()),
                Need::Default(value) => write!(out, " [{}:{value}]", arg.name),
            };
        }
        out
    }

    /// The whole of `help <name>`.
    pub fn help_for(&self, level: Level) -> String {
        let mut out = format!("{}\n{}", self.usage(level), self.summary);
        for arg in self.args_for(level) {
            let _ = write!(out, "\n  {}: {}", arg.name, arg.help);
            if let Some(range) = arg.kind.range_for(level) {
                let _ = write!(out, " ({} to {})", range.0, range.1);
            }
            if let Kind::Word(words) = arg.kind {
                let _ = write!(out, " ({})", words.join(", "));
            }
        }
        out
    }
}

impl Kind {
    fn placeholder(&self) -> &'static str {
        match self {
            Kind::Word(_) => "<word>",
            Kind::Number(_) => "<number>",
            Kind::Id => "<id>",
            Kind::Count(_) => "<count>",
            Kind::Text => "<text>",
        }
    }

    /// The widest range `level` reaches, or `None` for a kind that has none or a level that
    /// reaches none of them.
    fn range_for(&self, level: Level) -> Option<(f64, f64)> {
        if let Kind::Count(max) = self {
            return Some((0.0, *max as f64));
        }
        let Kind::Number(limits) = self else { return None };
        limits.iter().filter(|limit| level.at_least(limit.level)).fold(None, |range, limit| {
            Some(match range {
                None => (limit.min, limit.max),
                Some((min, max)) => (f64::min(min, limit.min), f64::max(max, limit.max)),
            })
        })
    }

    fn read(&self, arg: &'static ArgSpec, typed: &str, level: Level) -> Result<Value, BindError> {
        match self {
            Kind::Word(words) => words
                .iter()
                .find(|word| word.eq_ignore_ascii_case(typed))
                .map(|word| Value::Word(word))
                .ok_or_else(|| BindError::NotOneOf {
                    arg: arg.name,
                    value: typed.into(),
                    choices: words.join(", "),
                }),
            Kind::Number(_) => {
                // `f64::from_str` takes "inf" and "NaN", which nobody typed meaning a number.
                let n: f64 = typed
                    .parse()
                    .ok()
                    .filter(|n: &f64| n.is_finite())
                    .ok_or_else(|| BindError::NotANumber { arg: arg.name, value: typed.into() })?;
                let (min, max) = self.range_for(level).unwrap_or((f64::NAN, f64::NAN));
                if (min..=max).contains(&n) {
                    Ok(Value::Number(n))
                } else {
                    Err(BindError::OutOfRange { arg: arg.name, min, max })
                }
            }
            Kind::Id => {
                let parsed = match typed.strip_prefix("0x").or_else(|| typed.strip_prefix("0X")) {
                    Some(hex) => u64::from_str_radix(hex, 16),
                    None => typed.parse(),
                };
                parsed.map(Value::Id).map_err(|_| BindError::NotAnId { arg: arg.name, value: typed.into() })
            }
            Kind::Count(max) => match typed.parse::<u32>() {
                Ok(n) if n <= *max => Ok(Value::Count(n)),
                Ok(_) => Err(BindError::OutOfRange { arg: arg.name, min: 0.0, max: *max as f64 }),
                Err(_) => Err(BindError::NotACount { arg: arg.name, value: typed.into() }),
            },
            Kind::Text => Ok(Value::Text(typed.into())),
        }
    }
}

/// Fit a parsed line to `spec`, as `level` may use it.
pub fn bind(spec: &Spec, parsed: &Parsed, level: Level) -> Result<Bound, BindError> {
    let args: Vec<&'static ArgSpec> = spec.args_for(level).collect();
    // In the order typed, so `go 7 target:8` is a target given twice rather than a 7 that
    // quietly moved on to whatever came next.
    let mut typed: Vec<Option<&str>> = vec![None; args.len()];
    for given in &parsed.args {
        let at = match &given.key {
            Some(key) => args.iter().position(|arg| arg.name == key).ok_or_else(|| {
                BindError::UnknownArg { command: spec.name, arg: key.clone() }
            })?,
            None => typed.iter().position(Option::is_none).ok_or_else(|| BindError::TooMany {
                value: given.value.clone(),
                usage: spec.usage(level),
            })?,
        };
        if typed[at].replace(&given.value).is_some() {
            return Err(BindError::Repeated(args[at].name));
        }
    }

    let mut bound = Bound::default();
    for (arg, typed) in args.iter().zip(typed) {
        let typed = match (typed, arg.need) {
            (Some(typed), _) => typed,
            (None, Need::Default(value)) => value,
            (None, Need::Optional) => continue,
            (None, Need::Required) => {
                return Err(BindError::Missing { arg: arg.name, usage: spec.usage(level) });
            }
        };
        bound.values.push((arg.name, arg.kind.read(arg, typed, level)?));
    }
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::parse::parse;

    const LIMITS: &[Limit] = &[
        Limit { level: Level::DEBUG, min: 0.0, max: 10.0 },
        Limit { level: Level::SUPERADMIN, min: -5.0, max: 1000.0 },
    ];

    const ARGS: &[ArgSpec] = &[
        ArgSpec { name: "target", kind: Kind::Id, need: Need::Required, level: Level::PLAYER, help: "" },
        ArgSpec {
            name: "height",
            kind: Kind::Number(LIMITS),
            need: Need::Default("2"),
            level: Level::DEBUG,
            help: "",
        },
        ArgSpec {
            name: "mode",
            kind: Kind::Word(&["soft", "hard"]),
            need: Need::Optional,
            level: Level::DEBUG,
            help: "",
        },
        ArgSpec { name: "ship", kind: Kind::Id, need: Need::Optional, level: Level::SUPERADMIN, help: "" },
    ];

    const SPEC: Spec =
        Spec { name: "go", verb: Verb::Teleport, level: Level::PLAYER, summary: "", args: ARGS };

    fn bound(line: &str, level: Level) -> Result<Bound, BindError> {
        bind(&SPEC, &parse(line).expect("it parses"), level)
    }

    #[test]
    fn positionals_fill_what_was_not_named_in_order() {
        let got = bound("go 0x10 height:3 hard", Level::ADMIN).unwrap();
        assert_eq!(got.id("target"), Some(16));
        assert_eq!(got.number("height"), Some(3.0));
        assert_eq!(got.word("mode"), Some("hard"));
        assert_eq!(got.id("ship"), None);

        let got = bound("go 7 4", Level::ADMIN).unwrap();
        assert_eq!((got.id("target"), got.number("height")), (Some(7), Some(4.0)));
    }

    #[test]
    fn a_default_is_read_as_if_typed_and_an_optional_is_absent() {
        let got = bound("go 7", Level::DEBUG).unwrap();
        assert_eq!(got.number("height"), Some(2.0));
        assert_eq!(got.get("mode"), None);
    }

    /// An argument above the asker is an argument the command does not have: named, it is
    /// unknown, and positionally it is never reached.
    #[test]
    fn an_argument_above_the_asker_does_not_exist_for_them() {
        assert_eq!(
            bound("go 7 ship:9", Level::ADMIN),
            Err(BindError::UnknownArg { command: "go", arg: "ship".into() }),
        );
        assert!(matches!(bound("go 7 1 soft 9", Level::ADMIN), Err(BindError::TooMany { .. })));
        assert_eq!(bound("go 7 1 soft 9", Level::SUPERADMIN).unwrap().id("ship"), Some(9));
        // A player sees the target and nothing else, defaults included.
        let got = bound("go 7", Level::PLAYER).unwrap();
        assert_eq!(got.get("height"), None);
        assert!(!SPEC.usage(Level::PLAYER).contains("height"));
        assert!(SPEC.usage(Level::SUPERADMIN).contains("[ship:<id>]"));
    }

    /// A limit reaches its own level and everyone senior, and the widest reached applies.
    #[test]
    fn a_range_widens_with_seniority() {
        assert_eq!(
            bound("go 7 height:11", Level::ADMIN),
            Err(BindError::OutOfRange { arg: "height", min: 0.0, max: 10.0 }),
        );
        assert_eq!(bound("go 7 height:11", Level::SUPERADMIN).unwrap().number("height"), Some(11.0));
        assert!(bound("go 7 height:-1", Level::SUPERADMIN).is_ok());
        assert!(SPEC.help_for(Level::DEBUG).contains("(0 to 10)"));
    }

    #[test]
    fn a_value_that_is_not_its_kind_is_refused() {
        for (line, why) in [
            ("go seven", "an identifier"),
            ("go 7 height:NaN", "a number"),
            ("go 7 height:inf", "a number"),
            ("go 7 mode:sof", "one of soft, hard"),
            ("go 0xZZ", "an identifier"),
            ("go -1", "an identifier"),
        ] {
            let error = bound(line, Level::SUPERADMIN).expect_err(line).to_string();
            assert!(error.contains(why), "{line}: {error}");
        }
    }

    #[test]
    fn a_missing_or_repeated_argument_says_so() {
        assert!(matches!(bound("go", Level::ADMIN), Err(BindError::Missing { arg: "target", .. })));
        assert_eq!(bound("go 7 target:8", Level::ADMIN), Err(BindError::Repeated("target")));
        assert_eq!(bound("go target:8 target:8", Level::ADMIN), Err(BindError::Repeated("target")));
        // Named first, a positional moves on to the next argument instead.
        assert_eq!(bound("go target:8 3", Level::ADMIN).unwrap().number("height"), Some(3.0));
    }
}
