use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;
use silex_cli::{
    ExecutionConfig, JsonStorage, Playbook,
    playbook::{PlaybookBlock, PlaybookContract},
    xelis_common::crypto::Hash,
};

use super::modules::load_module;

type Fixture = ExecutionConfig;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileContract {
    contract: Hash,
    input: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePlaybook {
    fixture: PathBuf,
    #[serde(default)]
    contracts: Vec<FileContract>,
    #[serde(default)]
    scheduled_executions: bool,
    blocks: Vec<PlaybookBlock>,
}

pub(crate) struct LoadedPlaybook {
    pub playbook: Playbook,
    pub fixture: Fixture,
    pub fixture_path: PathBuf,
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("failed to open {}", path.display()))
}

fn resolve_path(directory: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        directory.join(path)
    }
}

pub(crate) fn load_fixture(path: &Path) -> Result<Fixture> {
    let path = absolute_path(path)?;
    let fixture: Fixture = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("invalid execution config {}", path.display()))?;
    fixture.storage.validate()?;
    Ok(fixture)
}

pub(crate) fn save_fixture(fixture: &Fixture, path: &Path) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(fixture)? + "\n")
        .with_context(|| format!("failed to write config {}", path.display()))
}

pub(crate) fn load_playbook(path: &Path) -> Result<LoadedPlaybook> {
    let path = absolute_path(path)?;
    let file: FilePlaybook = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("invalid playbook {}", path.display()))?;
    let directory = path.parent().context("playbook has no parent directory")?;
    let fixture_path = resolve_path(directory, file.fixture);
    let fixture = load_fixture(&fixture_path)?;
    let contracts = file
        .contracts
        .into_iter()
        .map(|contract| {
            let input = resolve_path(directory, contract.input);
            Ok(PlaybookContract {
                contract: contract.contract,
                module: load_module::<JsonStorage>(&input)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let playbook = Playbook {
        contracts,
        blocks: file.blocks,
        scheduled_executions: file.scheduled_executions,
    };
    playbook.validate()?;
    Ok(LoadedPlaybook {
        playbook,
        fixture,
        fixture_path,
    })
}
