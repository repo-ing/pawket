# Pawket product direction

## Goal

Turn a media link into a usable file with as little friction as possible. Use the same Rust engine everywhere so provider fixes benefit the web UI, extension, command line, API, and agents together.

## Name and visual direction

**Pawket** is a working product name, not a claim of domain or trademark availability. The visual system is a near-black canvas, hairline borders, a tight system sans (Inter when installed), white primary buttons and one periwinkle accent, in the spirit of Linear. The paw mark is the only illustration. The default flow stays short: link → available formats → save.

## What makes it useful

- Website: paste a link and choose an actual available file.
- Chrome: start from the current tab without copying its address manually.
- CLI: script a repeatable operation and receive JSON.
- MCP: let an agent inspect and save through bounded tools.
- API: use the same contract in a future desktop app, share sheet, or existing creative workflow.

There is no need for five extraction engines, a dashboard, an account system, or a token. New interfaces should call the shared engine.

## Locked first scope

The native Rust engine runs locally. The Chrome extension and local web page connect to it. The MCP client launches the same executable directly in stdio mode. The core saves original files by default. Locally configured FFmpeg adds explicit MP3 and MP4 encoding, while the Rust engine manages source inspection and downloads. Proxy-only environments are explicitly unsupported in this version.

The foundation implements direct public HTTPS files and ordinary HTML media. Service-specific social extraction is the next engineering milestone. A polished interface is not proof that every social site works.

## Provider graduation criteria

Before labeling any platform supported, verify representative live inputs against the pinned adapter version: ordinary video with sound, audio-only if offered, multiple quality variants, missing/deleted/private content, rate limits, expired URLs, and redirects. Confirm actual containers and tracks, not only successful HTTP responses. Retain deterministic parser fixtures and test the failure states. Do not promise audio extraction, MP3 conversion, or silent video until the corresponding media operation exists.

## Sensible order after v0.1

1. ~~One social provider with live fixtures and explicit support boundaries.~~ Done in v0.2: X posts.
2. HLS/DASH assembly where the Rust container implementation supports the source codecs and timing.
3. Persistent jobs with status/cancel, then right-click actions and background Chrome completion.
4. Remote API and streamable HTTP MCP only after deployment/authentication and abuse controls are concrete.
5. Desktop wrapper or OS share integration if daily use demonstrates the need.

Maintain the one-input web experience throughout. Keep advanced preferences in a small disclosure panel. Tool descriptions and schemas should match actual backend behavior.

## Current reference sources

- Cobalt source: https://github.com/imputnet/cobalt
- Official Rust MCP SDK: https://github.com/modelcontextprotocol/rust-sdk
- Chrome extension WASM/CSP: https://developer.chrome.com/docs/extensions/reference/manifest/content-security-policy
- Rust/WASM bindings: https://rustwasm.github.io/docs/wasm-bindgen/

These are architecture references. Pawket's current core is independently authored and does not invoke Cobalt or copy its extractor code.
