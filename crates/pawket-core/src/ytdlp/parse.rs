//! Pure translation between yt-dlp's JSON and Pawket's format model.
//!
//! Nothing here runs a process or touches the network, so every rule is
//! covered by fixture tests.

use super::offer::{self, Abilities, Track};
use crate::{network, MediaFormat, MediaInfo, PawketError, UnsupportedStream};
use serde::Deserialize;
use url::Url;

pub(crate) const PROVIDER: &str = "ytdlp";
/// Playlist entries listed per inspection.
pub(crate) const MAX_ENTRIES: usize = 50;
const MAX_TITLE_CHARS: usize = 200;
const MAX_MESSAGE_CHARS: usize = 200;

#[derive(Default, Deserialize)]
pub(crate) struct Info {
    #[serde(rename = "_type", default)]
    kind: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    webpage_url: Option<String>,
    #[serde(default)]
    is_live: Option<bool>,
    #[serde(default)]
    live_status: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    playlist_count: Option<u64>,
    #[serde(default)]
    extractor_key: Option<String>,
    #[serde(default)]
    formats: Option<Vec<Format>>,
    #[serde(default)]
    entries: Option<Vec<Option<Entry>>>,
    /// Single-file results put their only format at the top level.
    #[serde(flatten)]
    top: Format,
}

#[derive(Clone, Default, Deserialize)]
pub(crate) struct Format {
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    vcodec: Option<String>,
    #[serde(default)]
    acodec: Option<String>,
    #[serde(default)]
    height: Option<f64>,
    #[serde(default)]
    filesize: Option<f64>,
    #[serde(default)]
    filesize_approx: Option<f64>,
    #[serde(default)]
    abr: Option<f64>,
    #[serde(default)]
    tbr: Option<f64>,
}

#[derive(Deserialize)]
pub(crate) struct Entry {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    webpage_url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

impl Info {
    /// True when no site-specific extractor matched, only yt-dlp's generic
    /// page scan. Pawket's own providers are preferred for those links.
    pub(crate) fn is_generic(&self) -> bool {
        self.extractor_key.as_deref() == Some("Generic")
    }
}

fn codec_present(codec: &Option<String>) -> Option<bool> {
    codec.as_deref().map(|codec| codec != "none")
}

impl Format {
    fn has_video(&self) -> bool {
        codec_present(&self.vcodec).unwrap_or(self.height.is_some())
    }

    /// True only when the source states there is no video track.
    fn is_audio_only(&self) -> bool {
        self.vcodec.as_deref() == Some("none")
    }

    fn has_audio(&self) -> Option<bool> {
        codec_present(&self.acodec)
    }

    fn height(&self) -> Option<u32> {
        self.height
            .filter(|h| *h >= 1.0 && *h <= 10_000.0)
            .map(|h| h as u32)
    }

    fn audio_rate(&self) -> f64 {
        self.abr.or(self.tbr).unwrap_or(0.0)
    }

    fn size(&self) -> Option<u64> {
        self.filesize
            .or(self.filesize_approx)
            .filter(|size| *size > 0.0)
            .map(|size| size as u64)
    }

    fn extension(&self) -> Option<String> {
        self.ext.as_deref().and_then(clean_extension)
    }

    fn is_h264(&self) -> bool {
        self.vcodec.as_deref().is_some_and(|codec| {
            let codec = codec.to_ascii_lowercase();
            codec.starts_with("avc1") || codec.starts_with("avc3") || codec == "h264"
        })
    }

    fn track(&self) -> Track {
        Track {
            height: self.height(),
            has_video: self.has_video(),
            audio_only: self.is_audio_only(),
            has_audio: self.has_audio(),
            h264: self.is_h264(),
            extension: self.extension(),
            size: self.size(),
            audio_rate: self.audio_rate(),
        }
    }
}

fn clean_extension(ext: &str) -> Option<String> {
    let ext = ext.trim().to_ascii_lowercase();
    (!ext.is_empty() && ext.len() <= 5 && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then_some(ext)
}

fn clean_title(title: Option<&str>, fallback: &str) -> String {
    let title: String = title
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE_CHARS)
        .collect();
    if title.is_empty() {
        fallback.into()
    } else {
        title
    }
}

fn duration_ms(seconds: Option<f64>) -> Option<u64> {
    seconds
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| (s * 1000.0) as u64)
}

/// Turn yt-dlp's `-J` output for `source` into Pawket's inspection result.
pub(crate) fn to_media_info(
    source: &Url,
    info: Info,
    abilities: Abilities,
) -> crate::Result<MediaInfo> {
    let live = info.is_live == Some(true)
        || matches!(info.live_status.as_deref(), Some("is_live" | "is_upcoming"));
    if live {
        return Err(PawketError::SourceUnavailable(
            "This is a live or upcoming stream. Pawket can save it once it has ended.".into(),
        ));
    }
    let source_url = info
        .webpage_url
        .as_deref()
        .and_then(|url| network::validate_url(url).ok())
        .unwrap_or_else(|| source.clone());
    let title = clean_title(info.title.as_deref(), "media");
    let is_playlist = matches!(info.kind.as_deref(), Some("playlist" | "multi_video"));
    let (formats, unsupported) = if is_playlist {
        playlist_formats(&info, abilities)
    } else {
        (video_formats(&source_url, &info, abilities), Vec::new())
    };
    if formats.is_empty() {
        return Err(PawketError::Unsupported);
    }
    Ok(MediaInfo {
        source_url: source_url.to_string(),
        title,
        formats,
        provider: PROVIDER.into(),
        unsupported,
    })
}

fn video_formats(page: &Url, info: &Info, abilities: Abilities) -> Vec<MediaFormat> {
    let listed = info.formats.clone().unwrap_or_default();
    let tracks: Vec<Track> = if listed.is_empty() {
        vec![info.top.track()]
    } else {
        listed.iter().map(Format::track).collect()
    };
    let base = MediaFormat {
        url: page.to_string(),
        duration_ms: duration_ms(info.duration),
        ..MediaFormat::default()
    };
    offer::single(&base, &tracks, abilities)
}

fn playlist_formats(
    info: &Info,
    abilities: Abilities,
) -> (Vec<MediaFormat>, Vec<UnsupportedStream>) {
    let mut unsupported = Vec::new();
    let mut formats = Vec::new();
    let mut item = 0u32;
    let mut skipped = false;
    for entry in info.entries.iter().flatten().take(MAX_ENTRIES) {
        let Some(entry) = entry else {
            skipped = true;
            continue;
        };
        let link = entry.webpage_url.as_deref().or(entry.url.as_deref());
        let Some(url) = link.and_then(|link| network::validate_url(link).ok()) else {
            skipped = true;
            continue;
        };
        item += 1;
        let entry = offer::Entry {
            url: url.to_string(),
            title: clean_title(entry.title.as_deref(), &format!("Item {item}")),
            duration_ms: duration_ms(entry.duration),
        };
        formats.extend(offer::playlist_item(item, &entry, abilities));
    }
    if skipped {
        unsupported.push(UnsupportedStream {
            kind: "playlist_entry".into(),
            reason: "Some entries are private, removed or have no public link.".into(),
        });
    }
    let listed = info.entries.as_ref().map_or(0, Vec::len) as u64;
    if info.playlist_count.unwrap_or(listed) > MAX_ENTRIES as u64 {
        unsupported.push(UnsupportedStream {
            kind: "playlist".into(),
            reason: format!("Only the first {MAX_ENTRIES} entries are listed."),
        });
    }
    (formats, unsupported)
}

/// Bytes reported by one `PAWKET <downloaded> <total> <estimate>` line.
pub(crate) fn progress_line(line: &str) -> Option<(u64, Option<u64>)> {
    let mut parts = line.strip_prefix("PAWKET ")?.split_whitespace();
    let number = |value: Option<&str>| -> Option<u64> {
        value?
            .parse::<f64>()
            .ok()
            .filter(|n| *n >= 0.0)
            .map(|n| n as u64)
    };
    let downloaded = number(parts.next())?;
    let total = number(parts.next());
    let estimate = number(parts.next());
    Some((downloaded, total.or(estimate).filter(|n| *n > 0)))
}

/// Map yt-dlp's stderr to the error users see.
/// `has_cookies` says whether a login was configured, which changes the
/// advice for sign-in errors.
pub(crate) fn error_from(stderr: &str, has_cookies: bool) -> PawketError {
    let line = stderr
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("ERROR:"))
        .unwrap_or_else(|| {
            stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
        });
    let message = line.trim().trim_start_matches("ERROR:").trim();
    let reason = strip_extractor_prefix(message);
    // Match on yt-dlp's wording only: links it echoes (hosts like
    // drmusic.com, paths like /login) must not change the category.
    let lower: String = reason
        .split_whitespace()
        .filter(|word| !word.contains("://"))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .replace('’', "'");
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    if has(&["unsupported url"]) {
        return PawketError::Unsupported;
    }
    if has(&["larger than max-filesize", "max-filesize"]) {
        return PawketError::TooLarge {
            limit: crate::MAX_MEDIA_BYTES,
        };
    }
    if has(&["http error 429", "too many requests", "not a bot"]) {
        return PawketError::RateLimited;
    }
    if has(&["drm protected", "drm-protected", "drm protection"]) {
        return PawketError::SourceUnavailable(
            "This video is DRM-protected, so it can't be saved.".into(),
        );
    }
    if has(&[
        "sign in to",
        "log in to",
        "logged-in",
        "logged in",
        "login required",
        "requires login",
        "authentication is required",
        "account credentials",
        "private video",
        "video is private",
        "members-only",
        "members only",
        "confirm your age",
        "age-restricted",
        "age restricted",
    ]) {
        let advice = if has_cookies {
            "That one needs a signed-in account, and the configured login didn’t work for it. Sign in to the site in that browser (or refresh the cookies file) and try again."
        } else {
            "That one needs a signed-in account. Start Pawket with --cookies-from-browser <browser> to use your logins."
        };
        return PawketError::SourceUnavailable(advice.into());
    }
    if has(&["http error 404", "video unavailable", "does not exist"]) || lower == "not found." {
        return PawketError::Http(404);
    }
    let short: String = reason.chars().take(MAX_MESSAGE_CHARS).collect();
    if short.is_empty() {
        return PawketError::SourceUnavailable("Couldn’t read that link.".into());
    }
    PawketError::SourceUnavailable(format!("Couldn’t read that link: {short}"))
}

/// Drop yt-dlp's `[extractor] id: ` prefix from a message.
fn strip_extractor_prefix(message: &str) -> &str {
    let Some(rest) = message.strip_prefix('[') else {
        return message;
    };
    let Some((_, rest)) = rest.split_once("] ") else {
        return message;
    };
    rest.split_once(": ").map_or(rest, |(_, tail)| tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(json: &str) -> Info {
        serde_json::from_str(json).unwrap()
    }

    fn source() -> Url {
        Url::parse("https://www.youtube.com/watch?v=jNQXAC9IVRw").unwrap()
    }

    const VIDEO: &str = r#"{
        "_type":"video","title":"Me at the zoo","duration":19,
        "webpage_url":"https://www.youtube.com/watch?v=jNQXAC9IVRw",
        "formats":[
            {"format_id":"233","ext":"mp4","vcodec":"none","acodec":null},
            {"format_id":"139","ext":"m4a","vcodec":"none","acodec":"mp4a.40.5","abr":49.1,"filesize":117526},
            {"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a.40.2","abr":129.8,"filesize":309288},
            {"format_id":"251","ext":"webm","vcodec":"none","acodec":"opus","abr":106.0},
            {"format_id":"18","ext":"mp4","vcodec":"avc1","acodec":"mp4a.40.2","height":360},
            {"format_id":"160","ext":"mp4","vcodec":"avc1","acodec":"none","height":144},
            {"format_id":"137","ext":"mp4","vcodec":"avc1","acodec":"none","height":1080},
            {"format_id":"248","ext":"webm","vcodec":"vp9","acodec":"none","height":1080}
        ]}"#;

    fn ids(media: &MediaInfo) -> Vec<&str> {
        media.formats.iter().map(|f| f.id.as_str()).collect()
    }

    const MERGE: Abilities = Abilities {
        merge: true,
        opus: true,
    };

    #[test]
    fn reads_codecs_heights_and_titles_from_yt_dlp_json() {
        let media = to_media_info(&source(), info(VIDEO), MERGE).unwrap();
        assert_eq!(
            (media.provider.as_str(), media.title.as_str()),
            ("ytdlp", "Me at the zoo")
        );
        assert_eq!(
            &ids(&media)[..5],
            [
                "ytdlp-1080p",
                "ytdlp-1080p-mute",
                "ytdlp-360p",
                "ytdlp-360p-mute",
                "ytdlp-144p"
            ]
        );
        // H.264 exists at every height here, so nothing needs re-encoding.
        assert!(!ids(&media).iter().any(|id| id.ends_with("-h264")));
        let top = &media.formats[0];
        assert_eq!(top.url, "https://www.youtube.com/watch?v=jNQXAC9IVRw");
        assert_eq!(top.duration_ms, Some(19_000));
        assert!(top.choice.as_ref().unwrap().compatible);
        let audio = media
            .formats
            .iter()
            .find(|f| f.id == "ytdlp-audio-m4a")
            .unwrap();
        assert_eq!(audio.size_bytes, Some(309_288));
        assert!(ids(&media).contains(&"ytdlp-flac"));
    }

    #[test]
    fn without_ffmpeg_offers_only_files_that_already_have_sound() {
        let media = to_media_info(&source(), info(VIDEO), Abilities::default()).unwrap();
        assert_eq!(ids(&media), ["ytdlp-360p", "ytdlp-audio-m4a"]);
    }

    #[test]
    fn single_file_results_without_a_format_list_get_one_best_choice() {
        let json = r#"{"title":"clip","ext":"webm","url":"https://cdn.example.org/a.webm"}"#;
        let media = to_media_info(&source(), info(json), Abilities::default()).unwrap();
        assert_eq!(ids(&media), ["ytdlp-best"]);
        assert_eq!(media.formats[0].extension, "webm");
    }

    #[test]
    fn audio_only_sources_offer_audio_formats_but_no_video() {
        let json = r#"{"title":"song","formats":[
            {"format_id":"hls_opus","ext":"opus","vcodec":"none","acodec":"opus","abr":64},
            {"format_id":"http_mp3","ext":"mp3","vcodec":"none","acodec":"mp3","abr":128}]}"#;
        let media = to_media_info(&source(), info(json), MERGE).unwrap();
        assert_eq!(ids(&media)[0], "ytdlp-audio-mp3");
        assert!(media
            .formats
            .iter()
            .all(|f| f.kind == crate::MediaKind::Audio));
    }

    #[test]
    fn refuses_live_and_upcoming_streams() {
        for json in [r#"{"is_live":true}"#, r#"{"live_status":"is_upcoming"}"#] {
            assert!(matches!(
                to_media_info(&source(), info(json), MERGE),
                Err(PawketError::SourceUnavailable(_))
            ));
        }
    }

    #[test]
    fn playlists_become_numbered_items_and_skip_unsafe_links() {
        let json = r#"{"_type":"playlist","title":"Set","playlist_count":60,"entries":[
            {"url":"https://www.youtube.com/watch?v=MMxFDaIOHsE","title":"First","duration":775},
            null,
            {"url":"http://insecure.example.org/v","title":"Plain HTTP"},
            {"url":"https://127.0.0.1/v","title":"Local"},
            {"url":"https://www.youtube.com/watch?v=ps55secj7iU","title":"Second"}]}"#;
        let media = to_media_info(&source(), info(json), Abilities::default()).unwrap();
        assert_eq!(ids(&media), ["ytdlp-item-1-best", "ytdlp-item-2-best"]);
        assert_eq!(media.formats[1].label.as_deref(), Some("2. Second"));
        assert_eq!(
            media.formats[1].url,
            "https://www.youtube.com/watch?v=ps55secj7iU"
        );
        let kinds: Vec<_> = media.unsupported.iter().map(|u| u.kind.as_str()).collect();
        assert_eq!(kinds, ["playlist_entry", "playlist"]);
        let with_ffmpeg = to_media_info(&source(), info(json), MERGE).unwrap();
        assert_eq!(
            with_ffmpeg
                .formats
                .iter()
                .filter(|f| f.item == Some(2))
                .count(),
            9
        );
    }

    #[test]
    fn an_empty_playlist_is_unsupported() {
        let json = r#"{"_type":"playlist","entries":[]}"#;
        assert!(matches!(
            to_media_info(&source(), info(json), MERGE),
            Err(PawketError::Unsupported)
        ));
    }

    #[test]
    fn reads_progress_lines_with_missing_totals() {
        assert_eq!(
            progress_line("PAWKET 1024 4096 NA"),
            Some((1024, Some(4096)))
        );
        assert_eq!(
            progress_line("PAWKET 10.0 NA 2048.5"),
            Some((10, Some(2048)))
        );
        assert_eq!(progress_line("PAWKET 10 NA NA"), Some((10, None)));
        assert_eq!(progress_line("[download] 10%"), None);
        assert_eq!(progress_line("PAWKET NA NA NA"), None);
    }

    #[test]
    fn maps_yt_dlp_errors_to_clear_reasons() {
        let cases = [
            (
                "ERROR: Unsupported URL: https://example.com/",
                "unsupported_source",
            ),
            (
                "ERROR: [youtube] x: Sign in to confirm your age",
                "source_unavailable",
            ),
            (
                "WARNING: a\nERROR: HTTP Error 429: Too Many Requests",
                "rate_limited",
            ),
            ("ERROR: [x] 1: HTTP Error 404: Not Found", "http_error"),
            ("ERROR: This video is DRM protected", "source_unavailable"),
            ("ERROR: [dailymotion] x8j2s4e: Not found.", "http_error"),
            ("ERROR: [Reddit] 6rrwyj: Account authentication is required. Use --cookies-from-browser", "source_unavailable"),
            ("ERROR: [youtube] x: Sign in to confirm you’re not a bot", "rate_limited"),
            ("ERROR: [x] 1: File is larger than max-filesize (300 > 268435456)", "too_large"),
        ];
        for (stderr, code) in cases {
            assert_eq!(error_from(stderr, false).code(), code, "{stderr}");
        }
        // Hosts and paths in echoed links don't pick a category.
        for stderr in [
            "ERROR: [generic] a: Unable to download https://drmusic.example/login/private",
            "ERROR: [youtube] a: ffprobe and ffmpeg not found",
        ] {
            assert!(
                error_from(stderr, false)
                    .to_string()
                    .starts_with("Couldn’t read that link:"),
                "{stderr}"
            );
        }
        let login = "ERROR: [vimeo] 1: The web client only works when logged-in.";
        assert!(error_from(login, false)
            .to_string()
            .contains("--cookies-from-browser"));
        assert!(error_from(login, true)
            .to_string()
            .contains("configured login"));
        let other = error_from("ERROR: [vimeo] 123: Something odd happened", false);
        assert_eq!(
            other.to_string(),
            "Couldn’t read that link: Something odd happened"
        );
    }
}
