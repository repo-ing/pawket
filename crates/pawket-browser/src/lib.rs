//! Browser and Chrome-extension interaction logic, compiled from Rust to WASM.
//!
//! JavaScript entry points only load this module and bridge Chrome's active tab.
//! Resolving, validation, state transitions, DOM updates and downloads live here.

#[cfg(target_arch = "wasm32")]
mod browser;
pub mod picker;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Mount the web app or extension popup onto its accessible HTML shell.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start(surface: &str, current_url: &str, auto_inspect: bool) -> Result<(), JsValue> {
    browser::start(surface, current_url, auto_inspect)
}

/// The UI is intentionally inert on native targets, allowing workspace checks.
#[cfg(not(target_arch = "wasm32"))]
pub fn start(_surface: &str, _current_url: &str, _auto_inspect: bool) {}
