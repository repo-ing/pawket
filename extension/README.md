# Pawket Chrome extension

This Manifest V3 popup shares the Rust/WASM UI crate with the web app. JavaScript only loads the packaged WASM and passes in the active tab URL. It has no content script, background scraper, cloud API, or remotely loaded executable code.

## Build and load

Build `crates/pawket-browser` for `wasm32-unknown-unknown`, run `wasm-bindgen --target web`, and put its generated files in `extension/pkg/`. The root build task handles both the web and extension copies.

1. Start the local engine with `pawket serve`.
2. Open the engine's web interface and open **Connection** to copy its session token.
3. Open `chrome://extensions`, turn on **Developer mode**, choose **Load unpacked**, and select this `extension` directory.
4. Open the Pawket popup, expand **Connection**, paste the token, and save the connection.
5. Open a supported media page or direct media URL and click Pawket. Inspect the link, choose an actual offered file, and save it.

The session token changes when the engine restarts. Pair again with the new token. Endpoint and token are stored in this extension's local browser storage. The popup only accepts `http://127.0.0.1:3987` or `http://localhost:3987`, matching its declared host permissions.

## Permissions

- `activeTab`: read the current page's URL when the user invokes the popup.
- `downloads`: hand a finished local-engine file to Chrome's download manager, with a Save As prompt.
- Loopback host access on port 3987: inspect links and save files through the local Rust engine.

Chrome's download API reports that a download was accepted, not that it finished. Pawket tells the user to check Chrome's Downloads and keeps a file link available as a fallback.

Keep the popup open until the engine returns a result. This version has no background job notification. Supported sources and formats are determined by the engine; the extension does not add platform support or transcoding.
