//! One media engine shared by Pawket's CLI, HTTP API, and MCP server.
//!
//! Supports public HTTPS media, HTML media and public X posts. The default
//! saves source bytes, with explicit conversions if a local encoder is enabled.

mod choice;
mod convert;
mod network;
mod parse;
mod providers;
mod storage;
mod x;
mod youtube;
mod ytdlp;

pub use choice::{AudioFormat, Choice};
pub use ytdlp::CookieSource;

use futures_util::StreamExt;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use url::Url;

pub const MAX_HTML_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    #[default]
    Video,
    Audio,
    Image,
}

/// A source file or an explicit conversion of a source file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MediaFormat {
    pub id: String,
    pub url: String,
    pub extension: String,
    pub mime_type: String,
    pub kind: MediaKind,
    pub size_bytes: Option<u64>,
    /// Short human-readable description, e.g. "1280×720 · 2.2 Mbps".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Bits per second as reported by the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// `Some(false)` when the source states the file has no audio track
    /// (video-only). `None` means the source does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
    /// Which media item of a multi-item post this file belongs to (1-based).
    /// Formats sharing an `item` are alternative qualities of the same media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion: Option<Conversion>,
    /// Additional clear audio track when joining a YouTube MP4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_source_url: Option<String>,
    /// What this format is, for pickers: video or audio, quality, sound and
    /// audio format. Filled in for every format `Engine::inspect` returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<Choice>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Conversion {
    Mp3,
    /// H.264/AAC MP4 that plays in every common player.
    Mp4,
    Wav,
    Flac,
    Opus,
    /// The video track alone, copied without re-encoding.
    Mute,
}

/// A stream the source offers that this version cannot save, reported so
/// clients can explain it instead of silently hiding it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedStream {
    /// Stream type, e.g. "hls".
    pub kind: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MediaInfo {
    pub source_url: String,
    pub title: String,
    pub formats: Vec<MediaFormat>,
    /// Provider that produced this result: `direct_https`, `html_media` or `x`.
    #[serde(default)]
    pub provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<UnsupportedStream>,
}

/// Shared byte counters for one transfer. Cheap to read from another task.
#[derive(Debug, Default)]
pub struct Progress {
    written: AtomicU64,
    /// Zero means the source did not state a size.
    total: AtomicU64,
}

impl Progress {
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    /// The full size, when the source stated one.
    pub fn total(&self) -> Option<u64> {
        Some(self.total.load(Ordering::Relaxed)).filter(|total| *total > 0)
    }

    fn set_written(&self, bytes: u64) {
        self.written.store(bytes, Ordering::Relaxed);
    }

    fn set_total(&self, bytes: Option<u64>) {
        self.total.store(bytes.unwrap_or(0), Ordering::Relaxed);
    }
}

/// Providers this build implements, in the order they are tried.
pub const PROVIDERS: [&str; 3] = ["direct_https", "html_media", "x"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadResult {
    pub filename: String,
    pub path: String,
    pub bytes: u64,
    pub source_url: String,
}

#[derive(Debug, Error)]
pub enum PawketError {
    #[error("That link doesn’t look right. Use a full link starting with https://")]
    InvalidUrl,
    #[error("Pawket only saves from public websites, so it can’t open that link.")]
    BlockedAddress,
    #[error(
        "This network needs a proxy, which Pawket doesn’t support yet. Try a different network."
    )]
    ProxyUnsupported,
    #[error("Couldn’t reach that site. Check the link and your connection. ({0})")]
    Network(String),
    #[error("That site answered with an error ({0}). The link may be wrong or private.")]
    Http(u16),
    #[error("That link redirects too many times to follow.")]
    RedirectLimit,
    #[error("No files found on that page. Try a direct link to the video, audio or image.")]
    Unsupported,
    #[error("That file is no longer listed. Find the link’s files again and pick one.")]
    FormatUnavailable,
    #[error("That’s too big for Pawket. The limit here is {} MB.", .limit / (1024 * 1024))]
    TooLarge { limit: u64 },
    #[error("That link didn’t return a video, audio or image file.")]
    NotMedia,
    #[error("This format needs a configured FFmpeg executable with fd protocol support.")]
    ConverterUnavailable,
    #[error("Couldn’t convert this file. Check that the source has the required tracks and try saving the original.")]
    ConversionFailed,
    #[error("{0} links don’t work with Pawket yet. X posts, direct file links and ordinary web pages do.")]
    UnsupportedProvider(&'static str),
    #[error("That site is asking us to slow down. Wait a minute and try again.")]
    RateLimited,
    #[error("{0}")]
    SourceUnavailable(String),
    #[error("Couldn’t save the file: {0}")]
    Storage(String),
    #[error("The configured yt-dlp didn’t run. Check the --yt-dlp path and try again.")]
    BackendUnavailable,
    #[error("Login cookies aren’t set up: {0}")]
    CookiesUnavailable(String),
}

impl PawketError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUrl => "invalid_url",
            Self::BlockedAddress => "blocked_address",
            Self::ProxyUnsupported => "proxy_unsupported",
            Self::Network(_) => "network_error",
            Self::Http(_) => "http_error",
            Self::RedirectLimit => "redirect_limit",
            Self::Unsupported => "unsupported_source",
            Self::FormatUnavailable => "format_unavailable",
            Self::TooLarge { .. } => "too_large",
            Self::NotMedia => "not_media",
            Self::ConverterUnavailable => "converter_unavailable",
            Self::ConversionFailed => "conversion_failed",
            Self::UnsupportedProvider(_) => "unsupported_provider",
            Self::RateLimited => "rate_limited",
            Self::SourceUnavailable(_) => "source_unavailable",
            Self::Storage(_) => "storage_error",
            Self::BackendUnavailable => "backend_unavailable",
            Self::CookiesUnavailable(_) => "cookies_unavailable",
        }
    }
}

impl From<std::io::Error> for PawketError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<reqwest::Error> for PawketError {
    fn from(error: reqwest::Error) -> Self {
        // Omit request URLs because query strings can contain signed tokens.
        Self::Network(error.without_url().to_string())
    }
}

pub type Result<T> = std::result::Result<T, PawketError>;

#[derive(Clone, Debug)]
pub struct Engine {
    ffmpeg: Option<PathBuf>,
    encoders: convert::Encoders,
    youtube_preview: bool,
    ytdlp: Option<ytdlp::Backend>,
}

impl Engine {
    pub fn new() -> Result<Self> {
        Ok(Self {
            ffmpeg: None,
            encoders: convert::Encoders::default(),
            youtube_preview: false,
            ytdlp: None,
        })
    }

    pub async fn with_ffmpeg(mut self, executable: PathBuf) -> Result<Self> {
        let (executable, encoders) = convert::check_executable(executable).await?;
        self.ffmpeg = Some(executable);
        self.encoders = encoders;
        Ok(self)
    }

    pub fn can_convert(&self) -> bool {
        self.ffmpeg.is_some()
    }

    pub fn with_youtube_preview(mut self, enabled: bool) -> Self {
        self.youtube_preview = enabled;
        self
    }

    pub fn youtube_preview_enabled(&self) -> bool {
        self.youtube_preview
    }

    /// Use a local yt-dlp executable for sites, streams and playlists the
    /// built-in providers do not cover.
    pub async fn with_ytdlp(mut self, executable: PathBuf) -> Result<Self> {
        self.ytdlp = Some(ytdlp::check_executable(executable).await?);
        Ok(self)
    }

    /// Sign yt-dlp runs in with a browser session or cookies file, so
    /// private, members-only and login-gated media can be saved.
    pub fn with_cookies(mut self, source: CookieSource) -> Result<Self> {
        let backend = self.ytdlp.take().ok_or_else(|| {
            PawketError::CookiesUnavailable("logins need the yt-dlp backend (--yt-dlp).".into())
        })?;
        self.ytdlp = Some(backend.with_cookies(source));
        Ok(self)
    }

    /// Which login source is configured: a browser name or "file".
    pub fn cookies_kind(&self) -> Option<String> {
        self.ytdlp
            .as_ref()
            .and_then(ytdlp::Backend::cookies)
            .map(CookieSource::kind)
    }

    /// Version of the configured yt-dlp, when one is configured.
    pub fn ytdlp_version(&self) -> Option<&str> {
        self.ytdlp.as_ref().map(ytdlp::Backend::version)
    }

    pub async fn warm_audio(&self) {
        if let Some(backend) = &self.ytdlp {
            backend.warm_audio().await;
        }
    }

    pub async fn mp3_selection(&self, input: &str) -> Result<(MediaInfo, MediaFormat)> {
        self.audio_selection(input, false).await
    }

    /// Resolve an audio shortcut. For individual YouTube videos
    /// the downloader can discover the title and audio in one invocation.
    pub async fn audio_selection(
        &self,
        input: &str,
        original: bool,
    ) -> Result<(MediaInfo, MediaFormat)> {
        if !original && !self.can_convert() {
            return Err(PawketError::ConverterUnavailable);
        }
        let source = network::validate_url(input)?;
        if self.ytdlp.is_some() && youtube::video_id(&source).is_some() {
            let base = MediaFormat {
                url: source.to_string(),
                kind: MediaKind::Audio,
                ..Default::default()
            };
            let format = if original {
                MediaFormat {
                    id: "ytdlp-quick-original".into(),
                    label: Some("Original audio".into()),
                    ..base
                }
            } else {
                MediaFormat {
                    id: "ytdlp-quick-mp3".into(),
                    conversion: Some(Conversion::Mp3),
                    label: Some("MP3 · 128 kbps".into()),
                    bitrate: Some(128000),
                    extension: "mp3".into(),
                    mime_type: "audio/mpeg".into(),
                    ..base
                }
            };
            let info = MediaInfo {
                source_url: source.to_string(),
                title: "YouTube audio".into(),
                provider: ytdlp::PROVIDER.into(),
                formats: vec![format.clone()],
                unsupported: vec![],
            };
            return Ok((info, format));
        }
        let info = self.inspect(input).await?;
        let mut format = info
            .formats
            .iter()
            .find(|f| {
                if original {
                    f.kind == MediaKind::Audio && f.conversion.is_none()
                } else {
                    f.conversion == Some(Conversion::Mp3) && f.bitrate == Some(128000)
                }
            })
            .ok_or(PawketError::FormatUnavailable)?
            .clone();
        if !original && info.provider != ytdlp::PROVIDER {
            format.id = "quick-mp3".into();
        }
        Ok((info, format))
    }

    /// Inspect a public HTTPS media file or an HTML page containing media.
    pub async fn inspect(&self, input: &str) -> Result<MediaInfo> {
        let mut info = self.inspect_originals(input).await?;
        // yt-dlp results already list their own conversions.
        if self.can_convert() && info.provider != ytdlp::PROVIDER {
            convert::add_formats(&mut info, self.encoders);
        }
        for format in info.formats.iter_mut().filter(|f| f.choice.is_none()) {
            format.choice = Some(choice::infer(format, &info.provider));
        }
        Ok(info)
    }

    fn abilities(&self) -> ytdlp::Abilities {
        ytdlp::Abilities {
            merge: self.can_convert(),
            opus: self.can_convert() && self.encoders.opus,
        }
    }

    /// Optional encoders found in the configured FFmpeg, e.g. `["opus"]`.
    pub fn optional_encoders(&self) -> Vec<&'static str> {
        self.encoders.opus.then_some("opus").into_iter().collect()
    }

    async fn inspect_originals(&self, input: &str) -> Result<MediaInfo> {
        let source = network::validate_url(input)?;
        if let Some(post_id) = x::post_id(&source) {
            let native = x::inspect(&source, &post_id).await;
            return match (&native, &self.ytdlp) {
                (Err(_), Some(backend)) => ytdlp::inspect(backend, &source, self.abilities())
                    .await
                    .map(|(info, _)| info)
                    .or(native),
                _ => native,
            };
        }
        let Some(backend) = &self.ytdlp else {
            return self.inspect_native(source).await;
        };
        if backend.cookies().is_none() {
            // A media extension is a hint. Let the native fetch confirm the
            // response before starting the general-purpose extractor.
            if parse::direct_format(&source, "", None).is_some() {
                if let Ok(info) = self.inspect_native(source.clone()).await {
                    if info.provider == "direct_https" {
                        return Ok(info);
                    }
                }
            }
            if let Some(id) = youtube::video_id(&source) {
                if let Ok(Some(info)) = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    youtube::inspect_catalog(&source, &id, self.abilities()),
                )
                .await
                {
                    return Ok(info);
                }
            }
        }
        let backend_error = match ytdlp::inspect(backend, &source, self.abilities()).await {
            Ok((info, false)) => return Ok(info),
            // Generic page scans: Pawket's own result keeps exact bytes and
            // its conversion choices, so it wins when it finds anything.
            Ok((info, true)) => return Ok(self.inspect_native(source).await.unwrap_or(info)),
            Err(error) => error,
        };
        match self.inspect_native(source).await {
            Ok(info) => Ok(info),
            // Prefer yt-dlp's reason unless it simply didn't recognise the link.
            Err(native) if matches!(backend_error, PawketError::Unsupported) => Err(native),
            Err(_) => Err(backend_error),
        }
    }

    async fn inspect_native(&self, source: Url) -> Result<MediaInfo> {
        if self.youtube_preview {
            if let Some(video_id) = youtube::video_id(&source) {
                return youtube::inspect(&source, &video_id, self.can_convert()).await;
            }
        }
        if let Some(name) = providers::unsupported_provider(&source) {
            return Err(PawketError::UnsupportedProvider(name));
        }
        let (final_url, response) = network::fetch(source).await?;
        let mime = normalized_mime(&response);
        let length = response.content_length();
        if let Some(format) = parse::direct_format(&final_url, &mime, length) {
            return Ok(MediaInfo {
                source_url: final_url.to_string(),
                title: parse::title_from_url(&final_url),
                formats: vec![format],
                provider: "direct_https".into(),
                unsupported: Vec::new(),
            });
        }
        if mime != "text/html" && mime != "application/xhtml+xml" {
            return Err(PawketError::Unsupported);
        }
        let data = network::read_limited(response, MAX_HTML_BYTES).await?;
        parse::parse_html(&final_url, &String::from_utf8_lossy(&data))
    }

    /// Download an inspected format, converting only when explicitly selected.
    /// Files are streamed and committed without replacing existing files.
    pub async fn download(
        &self,
        input: &str,
        directory: &Path,
        selection: Option<&str>,
    ) -> Result<DownloadResult> {
        let info = self.inspect(input).await?;
        let format = match selection {
            Some(id) => info.formats.iter().find(|format| format.id == id),
            None => info.formats.first(),
        }
        .ok_or(PawketError::FormatUnavailable)?;
        self.download_format(&info, format, directory).await
    }

    /// Download a previously inspected format. The URL and destination are
    /// revalidated; callers cannot use this method to bypass network checks.
    pub async fn download_format(
        &self,
        info: &MediaInfo,
        format: &MediaFormat,
        directory: &Path,
    ) -> Result<DownloadResult> {
        self.download_format_tracked(info, format, directory, &Progress::default())
            .await
    }

    /// Like `download_format`, and reports bytes written to `progress` as
    /// they arrive so another task can show how far along the transfer is.
    pub async fn download_format_tracked(
        &self,
        info: &MediaInfo,
        format: &MediaFormat,
        directory: &Path,
        progress: &Progress,
    ) -> Result<DownloadResult> {
        if info.provider == ytdlp::PROVIDER {
            let backend = self.ytdlp.as_ref().ok_or(PawketError::FormatUnavailable)?;
            return ytdlp::download(
                backend,
                self.ffmpeg.as_deref(),
                info,
                format,
                directory,
                progress,
            )
            .await;
        }
        let separate_audio = match &format.audio_source_url {
            Some(audio)
                if info.provider == "youtube" && format.conversion == Some(Conversion::Mp4) =>
            {
                Some(network::validate_url(audio)?)
            }
            Some(_) => return Err(PawketError::FormatUnavailable),
            None => None,
        };
        let conversion = format
            .conversion
            .map(|output| {
                self.ffmpeg
                    .as_deref()
                    .map(|ffmpeg| (output, ffmpeg))
                    .ok_or(PawketError::ConverterUnavailable)
            })
            .transpose()?;
        let source = network::validate_url(&format.url)?;
        let (final_url, response) = if info.provider == "youtube" {
            network::fetch_scoped(source, youtube::media_url_allowed).await?
        } else {
            network::fetch(source).await?
        };
        if separate_audio.is_none() {
            progress.set_total(response.content_length());
        }
        let mime = normalized_mime(&response);
        let actual = parse::direct_format(&final_url, &mime, response.content_length())
            .or_else(|| {
                if info.provider == "youtube"
                    && matches!(
                        mime.as_str(),
                        "" | "application/octet-stream" | "binary/octet-stream"
                    )
                {
                    parse::direct_format(&final_url, &format.mime_type, response.content_length())
                } else {
                    None
                }
            })
            .ok_or(PawketError::NotMedia)?;
        let (temp_path, written) = download_source(response, directory, progress, 0).await?;
        let audio_path = if let Some(audio) = separate_audio {
            let (_, response) = network::fetch_scoped(audio, youtube::media_url_allowed).await?;
            if !matches!(
                normalized_mime(&response).as_str(),
                "audio/mp4"
                    | "audio/x-m4a"
                    | ""
                    | "application/octet-stream"
                    | "binary/octet-stream"
            ) {
                return Err(PawketError::NotMedia);
            }
            Some(
                download_source(response, directory, progress, written)
                    .await?
                    .0,
            )
        } else {
            None
        };

        let (temp_path, extension, bytes) = match conversion {
            Some((output, ffmpeg)) => {
                progress.set_total(None);
                let kbps = format.bitrate.map(|bits| (bits / 1000) as u32);
                let (converted, bytes) = convert::run_with_effort(
                    ffmpeg,
                    &temp_path,
                    audio_path.as_deref(),
                    directory,
                    output,
                    kbps,
                    (format.id == "quick-mp3").then_some(7),
                )
                .await?;
                (converted, output.extension(), bytes)
            }
            None => (temp_path, actual.extension.as_str(), written),
        };
        let path = storage::commit_temporary(&temp_path, directory, &info.title, extension).await?;
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Ok(DownloadResult {
            filename,
            path: path.to_string_lossy().into_owned(),
            bytes,
            source_url: info.source_url.clone(),
        })
    }
}

async fn download_source(
    response: reqwest::Response,
    directory: &Path,
    progress: &Progress,
    already_written: u64,
) -> Result<(tempfile::TempPath, u64)> {
    let (mut file, path) = storage::create_temporary(directory).await?;
    let mut written: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(next) = stream.next().await {
        let chunk = next?;
        written = written.saturating_add(chunk.len() as u64);
        file.write_all(&chunk).await?;
        progress.set_written(already_written + written);
    }
    if written == 0 {
        return Err(PawketError::NotMedia);
    }
    file.sync_all().await?;
    Ok((path, written))
}

fn normalized_mime(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|header| header.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// URL syntax and literal-address validation without an outbound request.
/// DNS names are additionally resolved, validated and pinned before fetching.
pub fn validate_public_url(input: &str) -> Result<Url> {
    network::validate_url(input)
}

#[cfg(test)]
mod progress_tests {
    use super::Progress;

    #[test]
    fn reports_bytes_and_treats_a_missing_size_as_unknown() {
        let progress = Progress::default();
        assert_eq!((progress.written(), progress.total()), (0, None));
        progress.set_total(Some(100));
        progress.set_written(40);
        assert_eq!((progress.written(), progress.total()), (40, Some(100)));
        progress.set_total(None);
        assert_eq!(progress.total(), None);
    }
}
