use serde::{Deserialize, Serialize};

use crate::compiled::Compiled;
use crate::feature::{FeatureId, FeatureKind, Problem};

/// A named number, which later features read as `Var("name")`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: f64,
}

impl FeatureKind for Variable {
    const KIND: &'static str = "variable";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["name", "value"];

    fn new() -> Self {
        Self { name: "x".into(), value: 1.0 }
    }

    fn summary(&self) -> String {
        format!("{} = {}", self.name, self.value)
    }

    fn evaluate(&self, _: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let valid = self.name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && self.name.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !valid {
            return Err(Problem::Invalid(format!("{:?} is not a variable name", self.name)));
        }
        if out.variables.contains_key(&self.name) {
            return Err(Problem::Invalid(format!("{} is already defined", self.name)));
        }
        if !self.value.is_finite() {
            return Err(Problem::Invalid(format!("{} is not finite", self.name)));
        }
        out.variables.insert(self.name.clone(), self.value);
        Ok(())
    }
}
