use ahash::HashMap;
use nodejs_semver::Range;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub dependencies: HashMap<String, Range>,
}

