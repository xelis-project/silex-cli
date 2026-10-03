//! Filesystem access and file-format conversion used only by the CLI.

mod execution;
mod fixtures;
mod modules;

pub(super) use execution::load_execution;
pub(super) use fixtures::{load_fixture, load_playbook, save_fixture};
pub(super) use modules::{load_module, read_file, read_module, write_module, write_text};
