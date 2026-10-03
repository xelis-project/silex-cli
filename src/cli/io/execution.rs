use std::{fs, path::Path};

use anyhow::{Context, Result};
use silex_cli::RunOptions;

pub(crate) fn load_execution(path: &Path) -> Result<RunOptions> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read execution options {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid execution options {}", path.display()))
}
