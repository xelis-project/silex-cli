//! Compile and execute Silex contracts with caller-supplied modules, options, and storage.

pub mod contract;
pub mod playbook;
pub mod program;
pub mod runtime;
pub mod state;
pub mod storage;

pub use contract::compile_source;
pub use playbook::{BlockStorage, Playbook, PlaybookOutcome, PlaybookReport};
pub use program::{parse_argument, run_program};
pub use runtime::{Invocation, RunOptions, RunResult, Simulator};
pub use silex_types::{Primitive, ValueCell};
pub use storage::{ExecutionStorage, JsonStorage};
pub use xelis_common;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub mod wasm;
