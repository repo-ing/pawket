// Browser bootstrap only. Interaction and application logic are Rust/WASM.
import init, { start } from "./pkg/pawket_browser.js";

await init();
start("web", "", false);
