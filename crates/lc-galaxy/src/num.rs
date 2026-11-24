//! A number as written in a timeline: a literal, or a variable defined earlier in it.
//!
//! In the file a literal is a bare number and a variable is `Var("h_R")`. Variables resolve
//! once, when the timeline compiles, so nothing downstream of [`crate::Compiled`] sees a name.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, Deserializer, EnumAccess, VariantAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use crate::feature::Problem;

#[derive(Clone, Debug, PartialEq)]
pub enum Num {
    Lit(f64),
    Var(String),
}

impl Num {
    pub fn resolve(&self, variables: &BTreeMap<String, f64>) -> Result<f64, Problem> {
        match self {
            Num::Lit(v) => Ok(*v),
            Num::Var(name) => variables.get(name).copied().ok_or_else(|| Problem::UnknownVariable(name.clone())),
        }
    }

    pub fn variable(&self) -> Option<&str> {
        match self {
            Num::Lit(_) => None,
            Num::Var(name) => Some(name),
        }
    }
}

impl From<f64> for Num {
    fn from(v: f64) -> Self {
        Num::Lit(v)
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Num::Lit(v) => write!(f, "{v}"),
            Num::Var(name) => f.write_str(name),
        }
    }
}

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Num::Lit(v) => s.serialize_f64(*v),
            Num::Var(name) => s.serialize_newtype_variant("Num", 1, "Var", name),
        }
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Num;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a number or Var(\"name\")")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Num, E> {
                Ok(Num::Lit(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Num, E> {
                Ok(Num::Lit(v as f64))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Num, E> {
                Ok(Num::Lit(v as f64))
            }
            // RON's `deserialize_any` hands `Var("x")` over as a one-element sequence, with the
            // name dropped.
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Num, A::Error> {
                let name: String = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(0, &self))?;
                if seq.next_element::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::invalid_length(2, &self));
                }
                Ok(Num::Var(name))
            }
            fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Num, A::Error> {
                let (tag, variant): (String, _) = data.variant()?;
                if tag != "Var" {
                    return Err(de::Error::unknown_variant(&tag, &["Var"]));
                }
                Ok(Num::Var(variant.newtype_variant()?))
            }
        }
        d.deserialize_any(V)
    }
}
