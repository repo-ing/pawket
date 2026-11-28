# Local HTTP API

Base URL: `http://127.0.0.1:3987` by default. This release is local-only. The Rust server refuses a non-loopback bind address.

## Authentication and transport

`POST` operations require `Content-Type: application/json` and `X-Pawket-Token: <session pairing code>`. Copy the code through the local website's Connection panel. Codes change after server restarts. Do not hard-code a code into source or put it in a public link.

Same-origin pages and valid Chrome-extension origins are allowed through the CORS gate. A permitted Origin alone is not authentication. The Host header must match the loopback listener and port. Maximum JSON body size is 8 KiB.

## GET /health

Returns engine status and currently declared providers. This is not a live test of every provider.

```json
{"status":"ok","version":"0.2.0","providers":["direct_https","html_media","x"]}
```

## GET /v1/capabilities

Returns supported inputs, interfaces, limits, and preview providers enabled for this engine. `transcoding` and `conversion_outputs` reflect the local FFmpeg configuration.

## POST /v1/inspect

Input:

```json
{"url":"https://x.com/elonmusk/status/1585341984679469056"}
```

Response shape:

```json
{
  "source_url":"https://example.org/media/video.mp4",
  "title":"video",
  "formats":[
    {
      "id":"opaque-inspection-format-id",
      "url":"https://example.org/media/video.mp4",
      "extension":"mp4",
      "mime_type":"video/mp4",
      "kind":"video",
      "size_bytes":12345
    }
  ]
}
```

Optional format fields appear only when the source states them: `label` (e.g. `"1280×720"`, or `"Video 2 · 1280×720"` when a post has several items), `width`, `height`, `bitrate` (bits per second as reported by the source), `duration_ms`, and `has_audio`. `has_audio: false` means the file has no sound; its label then ends in `no sound`. A missing `has_audio` means the source does not say. `item` is the 1-based number of the photo or video a format belongs to in a multi-item post; formats sharing an `item` are alternative qualities of the same media, so "save all" means the first format of each `item`. `provider` is `direct_https`, `html_media`, `x`, or `youtube` when the preview is enabled. `unsupported` lists streams the source offers that this version cannot save, such as `{"kind":"hls","reason":"…"}`. Show those reasons; do not hide them.

A format with `conversion: "mp3"` or `conversion: "mp4"` requests conversion after the source file is downloaded. Its `url` is the input file URL. If a YouTube MP4 choice also has `audio_source_url`, the engine joins the two tracks into an MP4 without re-encoding. Output size is unknown until encoding finishes. Originals remain the first choices and the default.

Examples are schema illustrations, not a real source. Format IDs come from the engine; never synthesize or assume them. `size_bytes` can be null. `kind` is `video`, `audio`, or `image`. The title and URLs are untrusted source text; render text safely, not as raw HTML.

## POST /v1/download

```json
{"url":"https://example.org/media/video.mp4","format_id":"opaque-inspection-format-id"}
```

Response shape:

```json
{
  "id":"opaque-transfer-id",
  "filename":"video.mp4",
  "bytes":12345,
  "url":"/v1/files/opaque-transfer-id?ticket=opaque-download-ticket",
  "source_url":"https://example.org/media/video.mp4"
}
```

This means the local engine prepared the file. It does not mean the user's browser has completed saving its own copy. The web UI offers the returned attachment link; the Chrome popup hands it to the downloads API. Never label an accepted browser download as completed without observing completion.

HTTP format IDs are bound to a retained inspection plan for the exact source and selected media. Plans expire after 10 minutes; up to 64 are retained. A media URL can also expire between inspection and download. Handle an expired plan or unavailable media as an actionable failure and inspect again; do not silently substitute a format. Omit format_id for a one-shot operation that inspects and chooses the first format immediately.

For an audio track in its original format, use `"format_id":"quick-original"`. This requires an audio-only source track. For a one-step 128 kbps audio save, pass `"format_id":"quick-mp3"`. Pawket resolves the audio at download time for individual YouTube videos. For other supported links it inspects the source and chooses the first available 128 kbps MP3 conversion. The desktop shortcut uses a faster LAME encoding preset for individual YouTube videos. Selecting MP3 through the format picker uses the standard preset.

## POST /v1/jobs

Starts a download that keeps running after the request returns. Same body as `/v1/download`, plus an optional `client` tag (letters, digits and `-`, up to 24 characters; default `api`) that groups jobs by the surface that started them:

```json
{"url":"https://example.org/media/video.mp4","format_id":"opaque-inspection-format-id","client":"web"}
```

A bad link or unknown format is rejected here, before anything is queued. Otherwise the response is `{"id":"…","jobs":[…]}`. At most three transfers run at once; further jobs are `queued`. More than 12 queued or running jobs returns `busy`.

## GET /v1/jobs

Requires `X-Pawket-Token`. Returns jobs newest first:

```json
{"jobs":[{"id":"…","title":"Clip","source_url":"https://example.org/media/video.mp4","label":"1280×720","client":"web","status":"running","bytes_done":4063223,"bytes_total":14601970,"delivered":false}]}
```

`status` is `queued`, `running`, `done` or `failed`. `bytes_total` is `null` when the source states no size. A `done` job adds `filename` and `url` (the ticketed `/v1/files/…` link). A `failed` job adds `error` with `code` and `message`. Finished jobs expire with their file after 30 minutes, and the list keeps at most 30. Jobs are held in memory and do not survive an engine restart. Poll about twice a second while anything is active.

## POST /v1/jobs/{id}/delivered

Claims a finished file for one browser and returns `{"claimed":true}` to the first caller and `{"claimed":false}` afterwards. Claim first, then download, so two open windows don't both save the file. Unknown IDs return `job_not_found`.

## POST /v1/jobs/clear

Forgets finished jobs. Jobs still downloading are kept.

## GET /v1/history

Requires `X-Pawket-Token`. Returns the newest 50 downloads the engine prepared, newest first:

```json
{"entries":[{"title":"Clip","source_url":"https://example.org/media/video.mp4","filename":"video.mp4","bytes":12345,"saved_at":1794300000,"provider":"direct_https"}]}
```

`saved_at` is seconds since the Unix epoch. The list is stored in `history.json` in the engine's transfer folder. It records titles and source links, not file contents or where the browser saved them.

## POST /v1/history/clear

Requires `X-Pawket-Token`. Empties the history and returns `{"entries":[]}`.

## GET /v1/files/{id}?ticket=...

Retrieves a prepared file as `application/octet-stream` with an attachment filename. The ticket is a bearer capability. Keep it local and do not include it in analytics or shared logs. Links are session-scoped and expire after 30 minutes or when transfer copies are pruned or the process exits. Retention is capped at 20 files. Graceful shutdown removes only the server’s own temporary session directory.

## Errors

```json
{"error":{"code":"unsupported_source","message":"Actionable explanation"}}
```

Clients should show the message and use the code for branching. Codes include `pairing_required`, `invalid_url`, `blocked_address`, `unsupported_source`, `unsupported_provider` (unsupported providers and link types), `source_unavailable` (protected, deleted or media-less X posts), `rate_limited` (source returned HTTP 429), `format_unavailable`, `inspection_required`, `inspection_expired`, `too_large`, `not_media`, `converter_unavailable`, `conversion_failed`, and `proxy_unsupported`. Expected cases include pairing required, invalid/blocked URLs, unsupported inputs, unsupported proxy transport, unavailable formats, oversized files, storage failures, and a busy engine. The UI does not fake success or download progress percentages.

Jobs that survive an engine restart, cancelling a job, background notifications, remote authentication, streamable HTTP MCP, and cloud storage are outside v0.2.
