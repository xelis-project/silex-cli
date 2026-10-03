//! Browser transport bindings around the typed library API.
//! Modules use binary bytes; options, values, results, and snapshots use JSON.

mod error;

use anyhow::{Context, Result};
use wasm_bindgen::prelude::wasm_bindgen;
use xelis_common::{contract::ContractModule, crypto::Hash, serializer::Serializer};

use crate::{
    JsonStorage, RunOptions, Simulator, contract,
    program::{parse_argument as parse_value, run_program as execute_program},
};

pub use error::BrowserError;

type BrowserResult<T> = Result<T, BrowserError>;

fn decode_options(json: Option<&str>) -> Result<RunOptions> {
    serde_json::from_str(json.unwrap_or("{}")).context("invalid execution options JSON")
}

fn decode_module(bytes: &[u8]) -> Result<ContractModule> {
    ContractModule::from_bytes(bytes).context("invalid binary contract module")
}

fn decode_hash(hash: &str) -> Result<Hash> {
    Hash::from_hex(hash).context("invalid contract hash")
}

/// Compile source to a binary versioned module (Uint8Array in JavaScript).
#[wasm_bindgen(js_name = compileSource)]
pub fn compile_source(source: &str) -> BrowserResult<Vec<u8>> {
    let (module, _) = contract::compile_source(source)?;
    Ok(module.to_bytes())
}

/// Assemble text to a binary versioned module.
#[wasm_bindgen]
pub fn assemble(source: &str) -> BrowserResult<Vec<u8>> {
    Ok(contract::assemble(source)?.to_bytes())
}

/// Render a binary module as assembly.
#[wasm_bindgen]
pub fn disassemble(bytes: &[u8]) -> BrowserResult<String> {
    Ok(contract::disassemble(&decode_module(bytes)?)?)
}

/// Recover source from a binary module.
#[wasm_bindgen]
pub fn decompile(bytes: &[u8]) -> BrowserResult<String> {
    Ok(contract::decompile(&decode_module(bytes)?)?)
}

/// Generate an ABI JSON string from source.
#[wasm_bindgen(js_name = generateAbi)]
pub fn generate_abi(source: &str) -> BrowserResult<String> {
    Ok(contract::generate_abi(source)?)
}

/// Convert a CLI-style argument to its typed JSON encoding.
#[wasm_bindgen(js_name = parseArgument)]
pub fn parse_argument(argument: &str) -> BrowserResult<String> {
    Ok(serde_json::to_string(&parse_value(argument)?)?)
}

/// Run a standalone program, returning the value as JSON.
/// Only entry, arguments, and gas_limit from the options are used.
#[wasm_bindgen(js_name = runProgram)]
pub fn run_program(bytes: &[u8], options_json: Option<String>) -> BrowserResult<String> {
    let options = decode_options(options_json.as_deref())?;
    let value = execute_program::<JsonStorage>(
        &decode_module(bytes)?,
        options.entry,
        options.arguments,
        Some(options.gas_limit),
    )?;
    Ok(serde_json::to_string(&value)?)
}

/// Stateful browser simulator using the same contract runner as the Rust API.
/// Await each call before accessing this instance again.
#[wasm_bindgen(js_name = Simulator)]
pub struct BrowserSimulator {
    inner: Simulator,
}

#[wasm_bindgen(js_class = Simulator)]
impl BrowserSimulator {
    /// Create from a standalone storage JSON snapshot, or an empty default environment.
    #[wasm_bindgen(constructor)]
    pub fn new(storage_json: Option<String>) -> BrowserResult<BrowserSimulator> {
        let storage = match storage_json {
            Some(json) => serde_json::from_str(&json).context("invalid environment JSON")?,
            None => JsonStorage::default(),
        };
        Ok(Self {
            inner: Simulator::new(storage)?,
        })
    }

    /// Export a snapshot for persistence or another simulator instance.
    #[wasm_bindgen(js_name = storageJson)]
    pub fn storage_json(&self) -> BrowserResult<String> {
        Ok(serde_json::to_string_pretty(self.inner.storage())? + "\n")
    }

    /// Register a binary module under a 64-character hexadecimal contract hash.
    #[wasm_bindgen]
    pub fn register(&mut self, hash: &str, bytes: &[u8]) -> BrowserResult<()> {
        self.inner
            .register(decode_hash(hash)?, decode_module(bytes)?)?;
        Ok(())
    }

    /// Compile, register, and invoke source, resolving to a result JSON string.
    #[wasm_bindgen(js_name = runSource)]
    pub async fn run_source(
        &mut self,
        hash: String,
        source: String,
        options_json: Option<String>,
    ) -> BrowserResult<String> {
        let hash = decode_hash(&hash)?;
        let options = decode_options(options_json.as_deref())?;
        let result = self.inner.run_source(hash, &source, options).await?;
        Ok(serde_json::to_string(&result)?)
    }

    /// Invoke a registered contract, resolving to a result JSON string.
    #[wasm_bindgen]
    pub async fn run(
        &mut self,
        hash: String,
        options_json: Option<String>,
    ) -> BrowserResult<String> {
        let hash = decode_hash(&hash)?;
        let options = decode_options(options_json.as_deref())?;
        let result = self.inner.run(&hash, options).await?;
        Ok(serde_json::to_string(&result)?)
    }
}
