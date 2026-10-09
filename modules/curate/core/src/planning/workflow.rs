//! Pass dependencies, kept separate from the application's IO and progress.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pass {
    Describe,
    Fields,
    Organize,
}

impl Pass {
    pub fn dependencies(self) -> &'static [Self] {
        match self {
            Self::Describe => &[],
            Self::Fields => &[Self::Describe],
            Self::Organize => &[Self::Fields],
        }
    }
}

pub fn workflow() -> Vec<Pass> {
    // Deliberately not execution order: dependencies decide that.
    let mut pending = vec![Pass::Organize, Pass::Fields, Pass::Describe];
    let mut ordered = Vec::new();
    while !pending.is_empty() {
        let next = pending
            .iter()
            .position(|pass| pass.dependencies().iter().all(|p| ordered.contains(p)))
            .expect("curate pass dependencies must be acyclic");
        ordered.push(pending.remove(next));
    }
    ordered
}
