//! Features, the open set of kinds they come in, and the registry that reads them.
//!
//! After Century's `FeatureKind`: a kind is a params struct behind a trait, registered by its
//! name in the file, so an older build can carry a kind it has never heard of.

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt::{self, Debug};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::codec;
use crate::compiled::Compiled;

/// Never reused within a timeline, so a reference survives reordering and deletion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct FeatureId(pub u32);

impl fmt::Display for FeatureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// One of the outputs of a feature that makes several. `of` is how many there were when the
/// reference was made, so a spiral that goes from two arms to three makes "arm 2 of 2" name
/// nothing rather than the wrong arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Output {
    pub feature: FeatureId,
    pub index: u32,
    pub of: u32,
}

impl Output {
    pub fn only(feature: FeatureId) -> Self {
        Self { feature, index: 0, of: 1 }
    }
}

/// Why a feature did not evaluate. The rest of the timeline still does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A feature this one reads is absent, unread, later in the timeline, or failed itself.
    Missing(FeatureId),
    /// A feature this one reads is suppressed. Not held: suppressing is how a designer asks
    /// to see the galaxy without it.
    Suppressed(FeatureId),
    WrongKind { id: FeatureId, wanted: &'static str },
    UnknownVariable(String),
    Invalid(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Missing(id) => write!(f, "reads {id}, which did not evaluate"),
            Problem::Suppressed(id) => write!(f, "reads {id}, which is suppressed"),
            Problem::WrongKind { id, wanted } => write!(f, "{id} is not a {wanted}"),
            Problem::UnknownVariable(name) => write!(f, "no variable named {name} before this"),
            Problem::Invalid(why) => f.write_str(why),
        }
    }
}

pub trait FeatureKind: Clone + PartialEq + Debug + Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The name in the file. Never changed once shipped.
    const KIND: &'static str;
    /// Bumped when the params change shape; the old shape is read by `migrate`.
    const VERSION: u32;
    /// Every field the params serialize, for telling `Unsupported` apart.
    const FIELDS: &'static [&'static str];

    fn new() -> Self;
    fn depends_on(&self) -> Vec<FeatureId> {
        Vec::new()
    }
    /// Names of the variables this feature reads.
    fn variables(&self) -> Vec<String> {
        Vec::new()
    }
    fn summary(&self) -> String;
    /// Validate before writing anything into `out`: a feature that fails must leave no output.
    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem>;
    fn migrate(from: u32, _ron: &str) -> Result<Self, String> {
        Err(format!("{} has no migration from version {from}", Self::KIND))
    }
}

/// The object-safe face of a [`FeatureKind`].
pub trait Params: Debug + Send + Sync + Any {
    fn kind(&self) -> &'static str;
    fn version(&self) -> u32;
    fn depends_on(&self) -> Vec<FeatureId>;
    fn variables(&self) -> Vec<String>;
    fn summary(&self) -> String;
    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem>;
    fn to_ron(&self) -> String;
    fn clone_box(&self) -> Box<dyn Params>;
    fn eq_dyn(&self, other: &dyn Params) -> bool;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: FeatureKind> Params for T {
    fn kind(&self) -> &'static str {
        T::KIND
    }
    fn version(&self) -> u32 {
        T::VERSION
    }
    fn depends_on(&self) -> Vec<FeatureId> {
        FeatureKind::depends_on(self)
    }
    fn variables(&self) -> Vec<String> {
        FeatureKind::variables(self)
    }
    fn summary(&self) -> String {
        FeatureKind::summary(self)
    }
    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        FeatureKind::evaluate(self, id, out)
    }
    fn to_ron(&self) -> String {
        codec::to_ron(self)
    }
    fn clone_box(&self) -> Box<dyn Params> {
        Box::new(self.clone())
    }
    fn eq_dyn(&self, other: &dyn Params) -> bool {
        other.as_any().downcast_ref::<T>() == Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Clone for Box<dyn Params> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl PartialEq for Box<dyn Params> {
    fn eq(&self, other: &Self) -> bool {
        self.eq_dyn(other.as_ref())
    }
}

#[derive(Clone, Debug)]
pub struct Feature {
    pub id: FeatureId,
    pub enabled: bool,
    pub name: Option<String>,
    /// The group row this feature folds under in the timeline.
    pub group: Option<FeatureId>,
    pub params: Box<dyn Params>,
}

impl PartialEq for Feature {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id && self.enabled == o.enabled && self.name == o.name && self.group == o.group
            && self.params.eq_dyn(o.params.as_ref())
    }
}

impl Feature {
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.params.summary())
    }

    pub fn get<T: FeatureKind>(&self) -> Option<&T> {
        self.params.as_any().downcast_ref()
    }

    pub fn get_mut<T: FeatureKind>(&mut self) -> Option<&mut T> {
        self.params.as_any_mut().downcast_mut()
    }
}

pub(crate) struct Kind {
    pub version: u32,
    pub fields: &'static [&'static str],
    pub new: fn() -> Box<dyn Params>,
    pub read: fn(u32, &str) -> Result<Box<dyn Params>, String>,
}

/// The kinds this build can read, by their name in the file.
pub struct Registry {
    kinds: BTreeMap<&'static str, Kind>,
}

impl Registry {
    pub fn empty() -> Self {
        Self { kinds: BTreeMap::new() }
    }

    /// Every kind this crate defines.
    pub fn standard() -> Self {
        let mut r = Self::empty();
        crate::kinds::register(&mut r);
        r
    }

    pub fn register<T: FeatureKind>(&mut self) {
        fn read<T: FeatureKind>(version: u32, ron: &str) -> Result<Box<dyn Params>, String> {
            if version == T::VERSION {
                codec::options().from_str::<T>(ron).map(|t| Box::new(t) as Box<dyn Params>).map_err(|e| e.to_string())
            } else {
                T::migrate(version, ron).map(|t| Box::new(t) as Box<dyn Params>)
            }
        }
        self.kinds.insert(
            T::KIND,
            Kind { version: T::VERSION, fields: T::FIELDS, new: || Box::new(T::new()), read: read::<T> },
        );
    }

    pub fn kinds(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.kinds.keys().copied()
    }

    pub fn create(&self, kind: &str) -> Option<Box<dyn Params>> {
        self.kinds.get(kind).map(|k| (k.new)())
    }

    pub(crate) fn get(&self, kind: &str) -> Option<&Kind> {
        self.kinds.get(kind)
    }
}
