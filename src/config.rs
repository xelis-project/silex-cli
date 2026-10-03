//! Typed execution configuration supplied by library callers.

use serde::{Deserialize, Serialize};
use xelis_common::crypto::Hash;

use crate::{JsonStorage, RunOptions};

/// An execution request and its storage backend. `storage` is required in JSON.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfig<Storage = JsonStorage> {
    /// Contract to register or invoke; defaults to the zero hash for simple fixtures.
    #[serde(default = "Hash::zero")]
    pub contract: Hash,
    pub storage: Storage,

    #[serde(default)]
    pub execution: RunOptions,
}
