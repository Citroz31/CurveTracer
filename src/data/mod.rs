pub mod dataset;
pub mod deembed;
pub mod excel;
pub mod expr;
pub mod hash;
pub mod loader;
pub mod touchstone;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub use dataset::{Column, ColumnKind, Dataset, format_sig};

/// A user function: a computed column defined by an expression, available
/// in every data source that contains the referenced columns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserFunction {
    pub name: String,
    pub expr: String,
    pub enabled: bool,
}

impl Default for UserFunction {
    fn default() -> Self {
        UserFunction {
            name: String::new(),
            expr: String::new(),
            enabled: true,
        }
    }
}

/// Builds a dataset = base columns + user function columns.
/// Returns the dataset and, for every function, an optional error.
pub fn with_functions(base: &Dataset, funcs: &[UserFunction]) -> (Dataset, Vec<Option<String>>) {
    let mut ds = base.clone();
    ds.strip_formulas();
    let mut errors = Vec::with_capacity(funcs.len());
    for f in funcs {
        let name = f.name.trim();
        if !f.enabled || name.is_empty() || f.expr.trim().is_empty() {
            errors.push(None);
            continue;
        }
        if ds.columns.iter().any(|c| c.name == name) {
            errors.push(Some(format!("A column named '{name}' already exists")));
            continue;
        }
        match expr::parse(&f.expr).and_then(|e| expr::evaluate(&e, &ds)) {
            Ok(values) => {
                ds.columns.push(Arc::new(Column::from_values(
                    name.to_owned(),
                    values,
                    Some(f.expr.clone()),
                )));
                errors.push(None);
            }
            Err(e) => errors.push(Some(e)),
        }
    }
    ds.generation = next_generation();
    (ds, errors)
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Unique, increasing number identifying a dataset content.
pub fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed)
}
