# Verification: Pawket v0.2

Everything below was run on **2026-11-06 (America/Los_Angeles)** on the owner's Mac:

- macOS 26.5.1 (Darwin 25.5.0, arm64)
- Rust 1.95.0
- wasm-bindgen CLI 0.2.130 (installed into the ignored `.local/`)
- Direct HTTPS egress with no proxy variables set

Media validity was checked with `ffprobe` and by fully decoding each file with `ffmpeg -v error -i FILE -f null -`; every decode reported no errors. FFmpeg is a **test tool only**. Pawket does not call it.

A row marked **PENDING** has not been run, so do not read it as working.

## Build and automated tests

| Check | Result |
| --- | --- |
| `cargo test --workspace --locked` (v0.1 source, before changes) | Pass: 31 passed, 0 failed |
| `cargo run -p xtask -- web` (v0.1 source) | Pass. The rebuilt module has the same exports as the bundled v0.1 `pkg`. Bytes differ only because of compiler and symbol-mangling differences, so the bundled assets matched the v0.1 source. |
| `cargo fmt --all --check` (v0.2) | Pass |
| `cargo clippy --workspace --all-targets --locked` (v0.2) | Pass, 0 warnings |
| `cargo test --workspace --locked` (v0.2) | Pass: **52 passed**, 0 failed (33 core, 16 server, 3 xtask), plus 4 live tests ignored by default |
| `cargo test -p pawket-core --test live_x --locked -- --ignored --test-threads=1` | Pass: **4 passed** against real X posts |
| `cargo build --release --locked -p pawket` | Pass; `pawket --version` reports `pawket 0.2.0` |
| `cargo run --locked -p xtask -- web` (v0.2) | Pass. `web/pkg` and `extension/pkg` hold byte-identical WASM (sha1 `7c2ddbd8…`), committed with the source. |
| Independent code review (Rust reviewer agent) | No critical or high findings. The reviewer checked the base-36 token port against V8 on 20,000 random IDs with 0 mismatches. Its medium and low findings were fixed and covered by regression tests: X media must come from `twimg.com`; audio probes run concurrently with a 10 s cap; media entries are capped at 16; "no audio" needs codec info on every variant; malformed entries are skipped and reported; invalid usernames are not used in URLs; renditions without stated sizes keep distinct IDs; dropped media is listed under `unsupported`. |
| Bundled WASM loads in a browser | Pass. The website shows **Engine online** and renders format choices from the Rust/WASM client (in-app Chromium pane). |

The X regression fixtures in `crates/pawket-core/tests/fixtures/x/` are unedited responses from X's embed endpoint and HLS playlists, captured 2026-11-06. They cover a video with sound, a video without sound, a very short silent video, a GIF, a photo, a protected-account tombstone and a text-only post.

## Real downloads: direct and HTML media

| Interface | URL | Format | Result |
| --- | --- | --- | --- |
| CLI `get` | https://www.w3schools.com/html/mov_bbb.mp4 | `original` | Pass. 788,493 bytes, H.264 320×176 + AAC, 10.03 s |
| CLI `get` | https://www.w3schools.com/html/horse.mp3 | `original` | Pass. 28,915 bytes, MP3, 1.55 s. A second `get` saved `horse-mp3-1.mp3` with identical bytes and did not overwrite. |
| CLI `get` (HTML page) | https://commons.wikimedia.org/wiki/File:Big_Buck_Bunny_4K.webm | `source-3` | Pass. 30,448,117 bytes, VP9 426×240 + Opus, 634.6 s |
| CLI `get` | https://upload.wikimedia.org/…/Big_Buck_Bunny_4K.webm.360p.mpeg4.mov | `original` | **Failed: HTTP 429** from Wikimedia right after the 30 MB download above. Wikimedia was rate-limiting. Pawket now reports this as `rate_limited`. |
| HTTP API | https://www.w3schools.com/html/mov_bbb.mp4 | plan ID | Pass. The `/v1/files` attachment was byte-identical to the CLI download (sha1 `438ad3bf…`). |
| Website | https://commons.wikimedia.org/wiki/File:Big_Buck_Bunny_4K.webm | 3rd WEBM | Pass. Paste, inspect (4 formats), select, save, then "Download file · 30.4 MB". That link's file was byte-identical to the CLI download. |
| MCP | https://www.w3schools.com/html/horse.mp3 | default | Pass. 28,915 bytes saved in the configured output directory |

## Real downloads: X posts

| Interface | URL | Format | Result |
| --- | --- | --- | --- |
| CLI | https://x.com/elonmusk/status/1585341984679469056 | default (`x-1-1920x1080`) | Pass. 3,139,253 bytes, H.264 1920×1080 + AAC, 9.31 s |
| CLI | same | `x-1-480x270` | Pass. 206,267 bytes, H.264 480×270 + AAC. The file name got a `-1` suffix instead of overwriting. |
| CLI | https://twitter.com/NASAUniverse/status/1505976485538111504/video/1 | default (`x-1-1280x720`) | Pass. 14,601,970 bytes, H.264 1280×720, 60.0 s, **no audio track**. X's HLS playlist also has no audio, so Pawket labels it video-only. |
| CLI | https://x.com/HonkaiChibi/status/1779732036124557398 | default | Pass. 14,583 bytes, H.264 264×258, 1.43 s, no audio, labelled video-only |
| CLI | https://x.com/morecats4/status/2088592754830184499 | `x-1-gif` | Pass. 10,890 bytes, H.264 320×320, no audio, labelled "GIF 1 · 320×320 · video only, no audio" |
| CLI | https://x.com/NASAWebb/status/1546621080298835970 | `x-1-photo` | Pass. 216,152-byte JPEG, 1386×1415, from `?name=orig`. Re-run after the review fixes saved it as `NASAWebb-Sneak-a-peek-at-the-deepest-sharpest-…jpg`, with `&amp;` now decoded in the title |
| HTTP API | https://twitter.com/elonmusk/status/1585341984679469056?s=20 | plan ID, 2nd entry | Pass. 1,655,117 bytes, H.264 1280×720 + AAC, 9.31 s. All four formats reported `has_audio: true`, and HLS was listed under `unsupported`. |
| MCP | https://twitter.com/NASAUniverse/status/1505976485538111504 | `x-1-480x270` | Pass. 2,006,548 bytes, H.264 480×270, 60.0 s. All three formats reported `has_audio: false`. |
| Website | https://x.com/morecats4/status/2088592754830184499 | GIF | Pass. Inspected; the card read "GIF 1 · 320×320 · video only, no audio" |
| Website | https://x.com/elonmusk/status/1585341984679469056 | 640×360 | Pass. 4 labelled choices plus the HLS note. Saving produced "Download file · 583.9 KB", which served H.264 640×360 + AAC, 9.31 s. |

### X error handling (live, HTTP API and MCP)

| Input | Code | Message |
| --- | --- | --- |
| https://x.com/Support/status/479307198901026816 (protected account) | `source_unavailable` | "X won't show this post without signing in: You're unable to view this Post because this account owner limits who can view their Posts." |
| https://x.com/a/status/1 (does not exist) | `source_unavailable` | "This X post doesn't exist or was deleted." |
| https://x.com/jack/status/20 (text only) | `source_unavailable` | "This X post has no video, GIF or photo to save." |
| https://x.com/NASA (profile) | `unsupported_provider` | "X profile, search and non-post links are not supported yet…" |
| https://youtu.be/aqz-KE-bpKQ, https://www.youtube.com/watch?v=aqz-KE-bpKQ | `unsupported_provider` | "YouTube links are not supported yet…" (no request is sent to YouTube) |
| https://www.tiktok.com/@scout2015/video/6718335390845095173 | `unsupported_provider` | "TikTok links are not supported yet…" |

## HTTP API security checks (live server, v0.1 and v0.2)

| Request | Expected | Result |
| --- | --- | --- |
| `POST /v1/inspect` with no token | 401 `pairing_required` | Pass |
| Wrong token | 401 `pairing_required` | Pass |
| `Host: evil.example:3987` | 403 `invalid_host` | Pass |
| `Origin: https://evil.example` with a valid token | 403 `origin_denied` | Pass |
| `http://` URL | 422 `invalid_url` | Pass |
| `https://192.168.1.1/a.mp4` | 422 `blocked_address` | Pass |
| Malformed JSON | 400 `invalid_request` | Pass |
| Made-up `format_id` | 409 `inspection_required` | Pass |
| `/v1/files/{id}` with wrong ticket | 401 `invalid_ticket` | Pass |
| `/v1/files/{id}` with the right ticket | 200, `attachment`, `no-store`, `nosniff` | Pass |

## MCP stdio (v0.2 binary, real JSON-RPC over stdin/stdout)

1. `initialize` negotiated protocol `2025-11-25`; the server reported version `0.2.0`.
2. `tools/list` returned exactly `pawket_capabilities`, `pawket_download` and `pawket_inspect`.
3. `pawket_capabilities` reported providers `direct_https`, `html_media` and `x`.
4. `pawket_inspect` on an X post returned three labelled formats with `has_audio: false`.
5. `pawket_download` with `x-1-480x270` saved a valid MP4 to the configured `--output` directory.
6. `pawket_inspect` on a YouTube link returned `isError: true` with `unsupported_provider`.
7. stderr was empty, nothing but protocol messages went to stdout, and the process exited 0.

`docs/mcp-integration-results.json` is the v0.1 run on the original Linux build machine, kept as history.

## Real browser: Chrome extension and website save (2026-11-10)

Run in a throwaway Brave 1.98.63 profile (Chromium 156), launched with `--load-extension=extension` and driven over the Chrome DevTools Protocol. Google-branded Chrome 157 no longer honours `--load-extension`. The extension was the committed, unmodified `extension/` folder. The test harness was a scratch script and is not part of Pawket.

| Check | Result |
| --- | --- |
| Extension loads unpacked, and its toolbar popup opens (`chrome.action.openPopup` on the post's window) | Pass |
| Pairing through the popup's **Connection** panel with the website's pairing code | Pass; the badge changed from "Pairing needed" to **Engine online** |
| Inspect https://x.com/elonmusk/status/1585341984679469056 in the popup | Pass. 4 labelled choices (1920×1080 down to 480×270) plus the HLS note |
| Choose 1280×720, then **Save file** through `chrome.downloads` | Pass. The popup said "File handed to Chrome"; the browser download **completed**, 1,655,117 bytes, as `elonmusk-Entering-Twitter-HQ-let-that-sink-in.mp4` |
| Saved file plays with sound | Pass. H.264 1280×720 + **AAC stereo**, 9.31 s, full decode clean, audio mean −33.1 dB / peak −1.9 dB. Byte-identical to the HTTP API download. |
| Website in the same real browser: inspect https://x.com/morecats4/status/2088592754830184499, pick "GIF 1 · 320×320 · video only, no audio", **Save file**, then **Download file** | Pass. The browser download **completed**: `morecats4.mp4`, 10,890 bytes, H.264 320×320, no audio, as labelled |
| Current tab URL prefilled in the popup | **Owner check, one click.** Chrome grants `activeTab` only when a person clicks the toolbar icon (or uses a shortcut); a scripted `openPopup()` gets no grant, so the tab URL is hidden and the field is empty. The test therefore typed the URL. The prefill code is the two-line `chrome.tabs.query({active: true, currentWindow: true})` in `extension/bootstrap.js`. |

## History, Save all and right-click (2026-11-10)

Run in a throwaway Brave 1.98.63 profile (Chromium 156) with the committed `extension/` folder, driven over the Chrome DevTools Protocol. Automated tests: **58 passed** (34 core, 21 server, 3 xtask).

| Check | Result |
| --- | --- |
| Website **Save file** starts the download without a second click | Pass. `horse-mp3.mp3` completed (28,915 bytes); status read "Download started. If nothing happened, use the link below." |
| **Recent** list appears after a save and survives the engine's own storage | Pass. The entry appeared with size and date; the history store has restart, cap, clear and corrupt-file tests. |
| Clicking a Recent entry looks the link up again | Pass. The link field was refilled and the files listed. |
| **Clear** empties the list | Pass |
| **Save all** hidden when there is one file | Pass |
| **Save all N files** on the website | Pass, with a caveat. The button read "Save all 4 files", the status ended "4 files sent to your browser's Downloads", and 4 browser downloads completed. **Caveat:** no real multi-photo X post was found to test with, so the test marked the four qualities of one video as four items in the engine's reply. The downloads were real. The engine's item numbering for real multi-media posts is covered by a unit test only. |
| Extension service worker loads and registers the **Save with Pawket** menu item | Pass |
| The window the menu opens (`popup.html?url=…`) looks the link up on its own | Pass. 4 files listed for the X post with no clicks. |
| Clicking the actual right-click menu item | **Owner check.** A native context menu can't be clicked by script. |
| **Save all** inside the extension popup | Not run. It uses the same Rust code as the website, with `chrome.downloads` and no save dialog per file. |

## Background downloads with progress (2026-11-10)

Automated tests: **65 passed** (35 core, 27 server, 3 xtask). Browser checks ran in a throwaway Brave 1.98.63 profile (Chromium 156) with the committed `extension/` folder.

| Check | Result |
| --- | --- |
| Engine reports progress for a real job (`POST /v1/jobs`, then `GET /v1/jobs`) | Pass. An X video went `queued`, then `running` with `bytes_done` climbing 1.0, 4.1, 7.3, 9.5 MB of 14.6 MB, then `done` with a file link. |
| Website shows a live progress row | Pass. A 96.6 MB Wikimedia file showed "0% · 91.8 KB of 96.6 MB" moments after **Save file**, then "Saved · 96.6 MB". |
| The link bar stays usable while a download runs | Pass |
| The finished file reaches the browser on its own | Pass. The browser download completed at 96,645,673 bytes. |
| Extension popup closed mid-download | Pass. With no window open, the engine finished the job (`done`, 96,645,673 bytes, not yet delivered). |
| Reopening the popup collects the file, once | Pass. One browser download completed on reopen and no second copy followed. |
| Queueing more than three downloads | Covered by the job store's unit tests; not run in a browser. |
| A job failing mid-transfer shows its error in the row | Covered by unit tests; not run in a browser. |

## Friendlier wording (2026-11-10)

All user-facing text was rewritten in plain language: "engine" became "Pawket", "inspect" became "find", "token" became "pairing code", and file rows show the size (`1280×720 · no sound`) and file type (`MP4 video`) instead of bitrates and MIME types. Error codes are unchanged; only messages changed. Messages quoted in the earlier sections of this file are the wording at the time of those runs.

Checked live on the website: the status badge read "Ready", an X post read "Found 3 files. Pick one to save.", and the errors for a YouTube link, an `http://` link, a private address, a missing X post and a page with no media all came back in the new wording. Automated tests: 65 passed.

## Known limits

- X can change, block or rate-limit its public embed endpoint at any time. The live tests above are the way to recheck it.
- `size_bytes` is `null` for X formats because X doesn't state file sizes. Pawket does not guess.
- For X videos, `has_audio` comes from X's HLS master playlist. If that playlist can't be read, `has_audio` is left out (unknown), not guessed.
- Wikimedia returned HTTP 429 after a large download in quick succession. Pawket reports it and doesn't retry.
- Background jobs live in memory: restarting the engine drops downloads in progress. There is no cancel button yet.
- A finished file is handed to the browser only while a Pawket page or popup is open. If none is open, it waits (up to 30 minutes) for the next one.
- Proxy-only networks still get `proxy_unsupported` rather than a silent bypass.
