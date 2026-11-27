<p align="center"><img src="assets/brand/pawket-logo-320.png" alt="Pawket logo: a black cat peeking out of a pocket with a download arrow" width="150"></p>

<h1 align="center">Pawket</h1>

<p align="center"><strong><a href="https://github.com/repo-ing/pawket/releases/latest/download/Pawket-for-Mac.zip">Download Pawket for Mac (M1 or newer)</a></strong></p>

On an Apple silicon Mac running macOS 14 or later, open the ZIP in Downloads and double-click **Pawket**. It opens in your browser. The app includes the tools for MP3 and MP4. To open Pawket again, click its Dock icon.

This Mac preview is not notarized. If macOS blocks the first launch, follow [Apple’s instructions for opening an app from a known source](https://support.apple.com/en-us/102445).

[Windows, Linux and Intel Mac downloads](https://github.com/repo-ing/pawket/releases/latest)

![Pawket: paste a link, pick Video or Audio and the quality, save](docs/pawket-demo.gif)

**Paste a link. Keep the file.** Pawket saves videos, music and photos from links: YouTube, TikTok, X, SoundCloud, Vimeo, playlists and most web pages. It runs on your own computer, with no account, ads or tracking, and nothing passes through someone else's server.

You choose what you get:

- **Video** in any quality the site offers, up to 4K, with or without sound. **Plays everywhere** converts it to H.264 so QuickTime, phones and editors can open it.
- **Audio** as MP3 (320, 192 or 128 kbps), WAV, FLAC, Opus or the original track.
- **Playlists**: tick the entries you want and save them all in one go.

The same Rust engine works in five places: a local website, a Chrome extension (right-click, **Save with Pawket**), the command line, AI agents over MCP, and an HTTP API.

## Install

**macOS and Linux:**

```sh
curl -fsSL https://raw.githubusercontent.com/repo-ing/pawket/main/install.sh | sh
pawket serve --open
```

**Windows:** download `pawket-windows-x64.zip` from [Releases](https://github.com/repo-ing/pawket/releases/latest), extract it and double-click **Start Pawket.cmd**.

Pawket opens at **http://127.0.0.1:3987**. Paste a link, press **Find media**, pick Video or Audio, and press **Save**. Keep the terminal open while you use it.

On its own Pawket saves X posts, direct file links and media on ordinary web pages. If you downloaded the Apple silicon app above, your media tools are already bundled. For the CLI and other platform ZIPs, to enable YouTube, TikTok, SoundCloud, Vimeo, playlists and the audio formats, install [FFmpeg](https://ffmpeg.org/) and [yt-dlp](https://github.com/yt-dlp/yt-dlp). Pawket finds them on your PATH by itself (`--no-detect` turns that off).

| | Command |
| --- | --- |
| macOS | `brew install ffmpeg yt-dlp deno` |
| Windows | `winget install Gyan.FFmpeg yt-dlp.yt-dlp DenoLand.Deno` |
| Linux | FFmpeg from your package manager, plus `pipx install yt-dlp` |

Deno lets yt-dlp handle YouTube's player checks. Releases are not code-signed yet, so macOS and Windows may ask you to approve the program the first time.

## What works

| Source | Without extras | With FFmpeg and yt-dlp |
| --- | --- | --- |
| X posts (video, GIF, photos) | Every quality X offers, original files | Same, plus audio formats and video without sound |
| Direct file links, web pages with `<video>`/`<audio>` | Original file | Same, plus every audio format, no-sound and MP4 versions |
| YouTube videos, Shorts | Opt-in preview for videos with direct files | Every quality up to 4K, all audio formats |
| YouTube playlists and channels | No | First 50 entries as a checklist |
| TikTok, SoundCloud, Dailymotion, Bilibili and most other yt-dlp sites | No | Yes |
| Vimeo | No | Public videos through Vimeo's embeddable player, no login |
| Reddit, Instagram, private or members-only videos | No | Only with your browser's login (`--cookies-from-browser`) |
| DRM-protected video, live streams | No | No |
| Files over 256 MiB | No | No (size cap) |

### How X works

The X adapter (`crates/pawket-core/src/x/`) reads the post through X's public embed endpoint, `cdn.syndication.twimg.com/tweet-result`. This is the same unauthenticated JSON that powers embedded posts on other websites. It needs no login, cookies, API key or third-party service. Media URLs on `video.twimg.com` and `pbs.twimg.com` go through the same HTTPS, public-address, redirect and size checks as every other download. X can change or rate-limit this endpoint at any time. When X rate-limits it, Pawket reports `rate_limited` rather than retrying silently.

### Everyday shortcuts

- **Recent.** The website lists your last downloads. Click one to look the link up again. **Clear** empties the list. The list lives in `history.json` in the engine's transfer folder and stores titles and links only.
- **Several files at once.** When an X post has several photos or videos, or a link is a playlist, Pawket shows a checklist. Tick what you want and Save applies your chosen format to each.
- **Right-click.** With the extension installed, right-click any page, link, video or image and choose **Save with Pawket**. A small Pawket window opens with the link already looked up.
- **Background downloads with progress.** **Save file** hands the download to the engine and shows a progress bar under the link bar. You can look up another link, or close the page or popup, while it runs. When it finishes, the file goes to your browser's Downloads. If the window was closed, that happens the next time you open it. Up to three downloads run at once; the rest wait their turn.

## Releases

Each release has ZIPs for macOS (Apple silicon and Intel), Linux and Windows, plus `SHA256SUMS`. A ZIP contains the `pawket` program, a double-click launcher, `START.md` and the Chrome extension. `install.sh` downloads the right ZIP, checks it against `SHA256SUMS` and links `pawket` into `~/.local/bin`.

Maintainers publish a release by pushing a version tag (`git tag v0.3.0 && git push origin v0.3.0`). The `Pawket builds` workflow builds and tests the four ZIPs and attaches them to a new GitHub Release. Every push also runs the `CI` workflow (tests and lint).

## Build from source

Install a current stable Rust toolchain from [rustup.rs](https://rustup.rs/) if needed (verified with Rust 1.95.0 on macOS 26.5.1, Apple silicon). The repository includes the generated browser assets (`web/pkg`, `extension/pkg`), so the first run needs only the native binary. From the project folder:

```sh
cargo run --release --locked -p pawket -- serve
```

Open **http://127.0.0.1:3987**. Paste an X post or another supported HTTPS link, inspect its actual formats, choose one, then prepare and save the file. Keep the terminal running. Stop with Control-C.

The website and extension use a local Rust process. This build is not a hosted cloud download service. The executable refuses non-loopback bind addresses. On a normal Mac network it uses direct HTTPS egress; proxy-only managed networks currently receive an explicit `proxy_unsupported` error rather than bypassing their proxy.

### CLI

```sh
cargo run --locked -p pawket -- capabilities
cargo run --locked -p pawket -- inspect 'https://x.com/elonmusk/status/1585341984679469056'
cargo run --locked -p pawket -- get 'https://x.com/elonmusk/status/1585341984679469056' --format x-1-1280x720 --output ./downloads
cargo run --locked -p pawket -- get 'https://www.w3schools.com/html/mov_bbb.mp4' --output ./downloads
```

`inspect` returns JSON. Copy a returned `formats[].id` into `get --format <id>`. Without `--format`, `get` takes the first format, which for X is the highest quality. CLI and MCP format IDs are stable (`x-1-1920x1080`, `x-1-gif`, `x-1-photo`, `source-2`, `original`). HTTP API IDs are opaque, per-inspection `plan_…` IDs. Originals preserve source bytes. Pawket never overwrites an existing file.
For a standalone executable:

```sh
cargo build --release --locked -p pawket
./target/release/pawket serve
```

The executable serves its embedded browser interface from any working directory. Pass `serve --open` to open the default browser after startup, or add `--assets /absolute/path/to/pawket/web` to use development assets.

### Audio and video formats

Set `PAWKET_FFMPEG` to the path of a local FFmpeg executable, or pass `--ffmpeg /path/to/ffmpeg`. It needs the `fd` protocol and the `libmp3lame`, `libx264`, `aac`, `flac` and `pcm_s16le` encoders; `libopus` adds Opus. The setting applies to the website, extension, CLI, MCP, and HTTP API.

The website shows a simple picker: **Video** or **Audio**, then the quality. Video comes with or without **Sound**, and **Plays everywhere** converts it to H.264/AAC MP4 when the original isn't already. Audio comes as the original track, **MP3** at 320, 192 or 128 kbps, **WAV**, **FLAC** or **Opus**. Every format also carries a `choice` object (`mode`, `height`, `sound`, `compatible`, `audio`, `kbps`) so other clients can build the same picker.

For direct files and X posts the CLI and MCP IDs are `original-as-mp3` (192 kbps), `original-as-mp3-320`, `original-as-mp3-128`, `original-as-wav`, `original-as-flac`, `original-as-opus`, `original-muted` and `original-as-mp4`. When a source has unknown audio status, audio conversions can fail if the file has no audio track. FFmpeg receives local, already downloaded file descriptors. Both input and output are bounded to 256 MiB.

### YouTube preview

Set `PAWKET_YOUTUBE_PREVIEW=true` or pass `--youtube-preview` to read direct files from a public YouTube video's player response. The same engine serves all five interfaces. Choose a combined MP4 where YouTube offers one. With FFmpeg enabled, Pawket can also join compatible direct MP4 video and M4A audio tracks or encode audio as MP3. Video-only originals are labeled **no sound**. The preview covers direct files and leaves player challenges, HLS/DASH, live streams, and playlists unsupported. Fixture tests cover the parser. Live compatibility still needs verification on a direct network.

### yt-dlp backend (all other sites)

Pass `--yt-dlp /path/to/yt-dlp` or set `PAWKET_YTDLP` to let a local [yt-dlp](https://github.com/yt-dlp/yt-dlp) handle links the built-in providers don't cover: YouTube (including player challenges; yt-dlp needs a JavaScript runtime such as Deno for this), TikTok, SoundCloud, Dailymotion, Bilibili and the rest of yt-dlp's supported sites, including HLS/DASH streams. With FFmpeg configured as well, Pawket offers every available height joined into one MP4 with sound (H.264/AAC preferred), the best audio file, and MP3.

```sh
pawket serve --ffmpeg /opt/homebrew/bin/ffmpeg --yt-dlp /opt/homebrew/bin/yt-dlp
```

- **Choices:** every video quality comes with sound, without sound (`-mute`) and, where the site has no H.264 at that height, re-encoded so it plays everywhere (`-h264`, slower). Audio comes as the original track (`ytdlp-audio-<ext>`), MP3 at 320/192/128 kbps (`ytdlp-mp3-320`, `ytdlp-mp3`, `ytdlp-mp3-128`), `ytdlp-wav`, `ytdlp-flac` and `ytdlp-opus` (when FFmpeg has libopus). Pawket's own FFmpeg step does every conversion. Direct files and X posts get the same audio choices plus `-muted`.
- **Playlists and channels** list their first 50 entries as numbered items with the same video and audio choices. The web page shows them as a checklist; Save applies the chosen format to every ticked entry.
- **Order:** X posts use the built-in adapter first (yt-dlp only if that fails). Other links go to yt-dlp first. When only yt-dlp's generic page scan matches, Pawket's own direct-file and HTML result is used if it finds anything, which keeps exact bytes.
- **Same limits:** every connection yt-dlp makes (redirects, embedded players, stream segments) goes through a loopback proxy inside Pawket that only allows HTTPS on port 443 to public addresses, resolved and pinned per connection. yt-dlp is limited to its native downloaders so no external tool fetches around the proxy. Files are capped at 256 MiB and nothing is overwritten. yt-dlp runs with `--ignore-config --no-plugin-dirs` and without `PYTHON*` variables, so local config can't add cookies, `--exec`, plugins or output paths (this needs a yt-dlp recent enough to have `--no-plugin-dirs`). Stopping a download stops yt-dlp and any FFmpeg it started.
- **Signed-in media:** add `--cookies-from-browser chrome` (or `firefox`, `safari`, `brave`, `edge`, `chromium`, `opera`, `vivaldi`, `whale`, optionally `:PROFILE`) to use that browser's logins, or `--cookies cookies.txt` for a Netscape-format cookies file. This covers private and members-only videos, age-restricted videos, and sites that block logged-out visitors, such as Reddit and Instagram. On macOS, Chromium browsers ask for Keychain access the first time; Safari needs Full Disk Access for the app running Pawket. Your logins then apply to every link Pawket opens, so only paste links you trust.
- **Vimeo without a login:** Vimeo links go through Vimeo's public embeddable player (`player.vimeo.com`), which still serves logged-out visitors. Videos Vimeo encrypts for that player are refused as DRM.
- **Not supported:** DRM and live streams.

## Chrome extension

The repository includes a ready-to-load `extension` directory with the compiled WASM asset. It has not been submitted to the Chrome Web Store.

1. Start `pawket serve` on the default port, 3987.
2. Open `chrome://extensions`, enable Developer mode, and choose **Load unpacked**.
3. Select this project's `extension` folder.
4. On the local Pawket website, click the **Ready** button at the top right, then **Copy code**.
5. Open the extension, open **Connection**, and save that pairing code.
6. Open an X post (or another supported page) and click the Pawket icon. The current tab URL is filled in. Inspect, choose a file, and save.

The pairing code changes when the native engine restarts. If replacing an earlier build, load the updated Pawket extension and pair it again; the application name and local pairing namespace changed. The extension accepts only `http://127.0.0.1:3987` or `http://localhost:3987`; these match its host permissions. It requests `activeTab`, `downloads` and `contextMenus` (for the right-click item), and does not run a content script over every page. If you loaded the extension before the right-click item existed, reload it at `chrome://extensions`. You can close the popup while a download runs; the engine finishes it, and the popup hands the file to Chrome the next time you open it. Downloads in progress do not survive an engine restart.

## Agent MCP

Pawket uses the official Rust MCP SDK, `rmcp`, and supports local stdio clients. The CLI binary is the MCP server; no separate web server is required for this mode.

Build the native executable, then add this configuration to your MCP client with actual absolute paths:

```json
{
  "mcpServers": {
    "pawket": {
      "command": "/absolute/path/to/pawket/target/release/pawket",
      "args": ["mcp", "--output", "/absolute/path/to/downloads"]
    }
  }
}
```

| Tool | Purpose |
| --- | --- |
| `pawket_capabilities` | Discover current support and limits |
| `pawket_inspect` | Resolve a URL (including an X post) into its available downloadable formats |
| `pawket_download` | Save the selected format to the configured output directory |

Example agent request: “Use Pawket to inspect this link, then save its original video to the configured downloads folder.” Tools return structured results and actionable errors. The agent cannot choose an arbitrary filesystem destination in a tool call; the MCP operator sets that directory at startup.

Stdio mode reserves stdout for MCP messages. Human diagnostics belong on stderr.

## HTTP API

The website and extension use the same local API. See [docs/API.md](docs/API.md) for payloads. API operations require a session pairing code in `X-Pawket-Token`; the native website receives its own code at render time.

## Rebuild the browser UI

This is needed only after changing the Rust UI code (`crates/pawket-browser`). Commit the regenerated `web/pkg` and `extension/pkg` with the change.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked --root .local
PATH="$PWD/.local/bin:$PATH" cargo run --locked -p xtask -- web
```

Installing into the ignored `.local` folder keeps this project's wasm-bindgen version from replacing a different version you use elsewhere.

The Rust `xtask` builds `pawket-browser`, generates its browser bindings, and copies the exact same compiled module into the web and extension folders. No Node package installation is required. Keep wasm-bindgen CLI and the lockfile's wasm-bindgen version aligned.

## Workspace

| Path | Responsibility |
| --- | --- |
| `crates/pawket-core` | URL policy, extraction (`parse.rs` HTML, `x/` X posts, `youtube.rs` preview), `ytdlp/` backend (guard proxy, offers, selection, logins, Vimeo route), `convert.rs` FFmpeg conversions, `choice.rs` picker descriptions, network streaming, filenames and atomic saves |
| `crates/pawket` | Native executable, local API/server, CLI, MCP |
| `crates/pawket-browser` | Shared Rust/WASM UI: `picker.rs` (DOM-free picker rules, unit tested), `browser/picker_view.rs` (the picker), state, validation and Chrome handoff |
| `crates/xtask` | Rust build orchestration |
| `web` | Website HTML/CSS and generated WASM assets |
| `extension` | Chrome MV3 manifest, popup shell and the same generated WASM assets |

## Boundaries

- HTTPS only, port 443. Credentials embedded in input URLs are rejected.
- Public destinations only. Local, private, link-local, reserved IP ranges and local hostname suffixes are rejected. yt-dlp's connections go through a loopback proxy in Pawket that applies the same rule to every connection.
- DNS results and every redirect are validated; direct connections use pinned addresses.
- HTML and X embed JSON are capped at 2 MiB, X HLS playlists (read only to detect audio) at 64 KiB, and downloaded files at 256 MiB.
- Original bytes are streamed to a temporary file and committed without overwriting an existing destination.
- The local HTTP API checks Host, Origin, and its pairing token; completed file links use unguessable download tickets.
- There are no analytics, external font requests, cloud accounts, or hard-coded API keys. Logins are opt-in: only `--cookies-from-browser` or `--cookies` hands yt-dlp your sessions.
- Outbound proxy transport is intentionally unsupported until it can preserve destination validation. Do not remove a required proxy to work around this limit.
- Rust/WASM is subject to browser platform rules. Chrome still requires a JSON manifest, HTML, and JavaScript entry-point/binding glue.

## Verification

Run:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked
PAWKET_TEST_FFMPEG=/path/to/ffmpeg cargo test -p pawket-core --locked converts_real_tracks_and_refuses_playlist_inputs -- --ignored
# Live yt-dlp download through the guard proxy:
PAWKET_TEST_YTDLP=/path/to/yt-dlp PAWKET_TEST_FFMPEG=/path/to/ffmpeg cargo test -p pawket-core --locked downloads_into_a_percent_directory_through_the_proxy -- --ignored
# Live direct-file YouTube preview check:
cargo test -p pawket-core --locked downloads_a_public_youtube_mp4 -- --ignored
# Live X checks (network required, opt-in):
cargo test -p pawket-core --test live_x --locked -- --ignored --test-threads=1
```

Offline tests use real X responses saved on 2026-11-06 in `crates/pawket-core/tests/fixtures/x/`. The live tests confirm X's endpoint still behaves the same way. What was actually tested, with URLs, dates and results, is in [docs/VERIFICATION.md](docs/VERIFICATION.md). Do not infer support for other platforms from a passing unit test.

## Next implementation milestone

Keep the UI small. Next candidates: a configurable size cap for long and 4K videos; recording live streams after they end; a visual check of the picker in the Chrome extension popup; a tested login flow with a real browser profile; job persistence before background Chrome notifications; and authenticated hosted operation before exposing the API publicly.

See [docs/PRODUCT.md](docs/PRODUCT.md) for the locked product direction.
