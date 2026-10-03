//! Convert Rust error chains to JavaScript Error objects at the browser boundary.

use anyhow::Error;
use js_sys::Error as JavaScriptError;
use serde_json::Error as JsonError;
use wasm_bindgen::JsValue;

/// Error returned by the browser bindings.
#[derive(Debug)]
pub struct BrowserError(Error);

impl From<Error> for BrowserError {
    fn from(error: Error) -> Self {
        Self(error)
    }
}

impl From<JsonError> for BrowserError {
    fn from(error: JsonError) -> Self {
        Self(error.into())
    }
}

impl From<BrowserError> for JsValue {
    fn from(error: BrowserError) -> Self {
        JavaScriptError::new(&format!("{:#}", error.0)).into()
    }
}
