// Chrome API bridge and WASM loading only. All app logic lives in Rust.
import init, { start } from "./pkg/pawket_browser.js";

await init();
// A right-click "Save with Pawket" opens this page with the chosen link.
const requested = new URLSearchParams(location.search).get("url");
if (requested) {
  start("extension", requested, true);
} else {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  start("extension", tab?.url ?? "", false);
}
