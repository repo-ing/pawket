Engine API implemented in src/lib.rs:
- Engine::new() -> Result<Engine, PawketError>; Engine is Clone.
- async inspect(&self, &str) -> Result<MediaInfo>
- async download(&self, &str, &Path, Option<&str>) -> Result<DownloadResult>
- async download_format(&self, &MediaInfo, &MediaFormat, &Path) -> Result<DownloadResult>
- validate_public_url(&str) -> Result<url::Url>
- error.code() -> &'static str; error Display is user-readable.
- MediaInfo { source_url: String, title: String, formats: Vec<MediaFormat>, provider: String, unsupported: Vec<UnsupportedStream> }
- MediaFormat { id, url, extension, mime_type, kind, size_bytes, plus optional label, width, height, bitrate, duration_ms, has_audio, conversion, audio_source_url }. Optional fields are omitted from JSON when unknown. has_audio == Some(false) means video-only.
- UnsupportedStream { kind: String, reason: String }: streams the source offers that are not downloadable (e.g. HLS).
- PROVIDERS: ["direct_https", "html_media", "x"]. X posts are routed to x/ before any generic fetch; Unsupported provider URLs fail early with unsupported_provider.
- MediaKind serde lower-case video/audio/image.
- DownloadResult { filename: String, path: String, bytes: u64, source_url: String }
Types support serde Serialize + Deserialize. inspect does not save. download streams into configured directory, original bytes only. Public HTTPS only. Direct URLs, HTML5 and OpenGraph advertised media, and public X posts via the syndication embed endpoint. When a local FFmpeg executable is configured, inspection appends explicit MP3 or MP4 choices. The optional YouTube preview reads direct public player URLs and applies a Google Video host policy to every media request.

SECURITY / TEST NOTE: core pins DNS and currently intentionally uses direct HTTPS, explicitly returning proxy_unsupported before any request when outbound proxy variables are configured (never silently bypasses them), because proxy DNS resolution cannot be pinned by reqwest resolve_to_addrs. Do not use live network tests from a proxy-mandated managed environment; offline tests cover core policy/parser/storage. Root should decide explicit honest proxy limitation or provide a controlled outbound proxy implementation.
