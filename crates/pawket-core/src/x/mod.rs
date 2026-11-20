//! X (Twitter) post adapter.
//!
//! Uses X's public, unauthenticated embed endpoint
//! (`cdn.syndication.twimg.com/tweet-result`), the same JSON that powers
//! embedded posts on other websites. It returns the post's media with direct
//! progressive MP4 variants on `video.twimg.com` and original photos on
//! `pbs.twimg.com`. No login, cookies, API key or third-party service is used.
//! Every returned URL is revalidated by the shared network policy.

use crate::{
    network, providers::host_matches, MediaFormat, MediaInfo, MediaKind, PawketError, Result,
    UnsupportedStream, MAX_HTML_BYTES,
};
use hls::playlist_has_audio;
use serde::Deserialize;
use token::syndication_token;
use url::Url;

mod hls;
mod token;

const POST_HOSTS: [&str; 2] = ["x.com", "twitter.com"];
const SYNDICATION_ENDPOINT: &str = "https://cdn.syndication.twimg.com/tweet-result";
const MAX_TITLE_CHARS: usize = 120;
const MAX_POST_ID_DIGITS: usize = 20;
const MAX_PLAYLIST_BYTES: u64 = 64 * 1024;
const NO_AUDIO: &str = "no sound";
/// X allows four media per post; anything far beyond that is not a real post.
const MAX_MEDIA_ENTRIES: usize = 16;
/// X serves post media only from its own CDN.
const MEDIA_DOMAIN: &str = "twimg.com";
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_KIND_CHARS: usize = 32;

/// The numeric post ID when `url` is an X/Twitter post link.
pub fn post_id(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    if !POST_HOSTS.iter().any(|domain| host_matches(host, domain)) {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let candidate = match segments.as_slice() {
        ["i", "web", "status", id, ..] | ["i", "status", id, ..] => *id,
        [_, "status" | "statuses", id, ..] => *id,
        _ => return None,
    };
    let is_numeric = !candidate.is_empty()
        && candidate.len() <= MAX_POST_ID_DIGITS
        && candidate.bytes().all(|b| b.is_ascii_digit());
    (is_numeric && candidate.parse::<u64>().is_ok()).then(|| candidate.to_owned())
}

/// Fetch and parse a post's media through the public embed endpoint.
pub async fn inspect(source: &Url, post_id: &str) -> Result<MediaInfo> {
    let mut endpoint = Url::parse(SYNDICATION_ENDPOINT).map_err(|_| PawketError::InvalidUrl)?;
    endpoint
        .query_pairs_mut()
        .append_pair("id", post_id)
        .append_pair("lang", "en")
        .append_pair("token", &syndication_token(post_id));
    let response = match network::fetch(endpoint).await {
        Err(PawketError::Http(404)) => {
            return Err(unavailable("That X post doesn’t exist, or it was deleted."))
        }
        other => other?.1,
    };
    let body = network::read_limited(response, MAX_HTML_BYTES).await?;
    let post: Post = serde_json::from_slice(&body)
        .map_err(|_| unavailable("X sent back something unexpected. Please try again later."))?;
    let media = decode_media(&post);
    let playlists = hls_playlists(&media);
    let mut info = parse_post(source, post_id, post)?;
    // Probes run together, each bounded, so a slow host can't stall inspect.
    let probes = playlists.into_iter().map(|(number, playlist)| async move {
        let result = tokio::time::timeout(PROBE_TIMEOUT, probe_audio(playlist)).await;
        (number, result.ok().flatten())
    });
    for (number, has_audio) in futures_util::future::join_all(probes).await {
        if let Some(has_audio) = has_audio {
            info = with_audio(info, number, has_audio);
        }
    }
    Ok(info)
}

/// Each media entry by 1-based number; `None` when the entry is unreadable.
/// One malformed entry must not make the whole post unreadable.
fn decode_media(post: &Post) -> Vec<(usize, Option<Media>)> {
    post.media_details
        .iter()
        .flatten()
        .take(MAX_MEDIA_ENTRIES)
        .enumerate()
        .map(|(index, raw)| (index + 1, Media::deserialize(raw).ok()))
        .collect()
}

/// A URL on X's media CDN that also passes the shared network policy.
fn media_url(raw: Option<&str>) -> Option<Url> {
    let url = network::validate_url(raw?).ok()?;
    host_matches(url.host_str()?, MEDIA_DOMAIN).then_some(url)
}

/// The HLS master playlist of each video, keyed by 1-based media number.
fn hls_playlists(media: &[(usize, Option<Media>)]) -> Vec<(usize, Url)> {
    media
        .iter()
        .filter_map(|(number, media)| {
            let media = media.as_ref().filter(|m| m.kind == "video")?;
            let variant = media
                .video_info
                .as_ref()?
                .variants
                .iter()
                .find(|v| v.is_hls())?;
            Some((*number, media_url(variant.url.as_deref())?))
        })
        .collect()
}

/// Whether a video has sound, read from its HLS master playlist. The MP4
/// variants are renditions of the same upload, so a playlist without audio
/// means the MP4s have none either. Any failure leaves it unknown.
async fn probe_audio(playlist: Url) -> Option<bool> {
    let (_, response) = network::fetch(playlist).await.ok()?;
    let body = network::read_limited(response, MAX_PLAYLIST_BYTES)
        .await
        .ok()?;
    playlist_has_audio(&String::from_utf8_lossy(&body))
}

/// Record a probed audio state on every rendition of media `number`.
fn with_audio(info: MediaInfo, number: usize, has_audio: bool) -> MediaInfo {
    let prefix = format!("x-{number}-");
    let formats = info
        .formats
        .into_iter()
        .map(|format| {
            if !format.id.starts_with(&prefix) || format.has_audio.is_some() {
                return format;
            }
            let label = match (has_audio, format.label) {
                (false, Some(label)) => Some(format!("{label} · {NO_AUDIO}")),
                (_, label) => label,
            };
            MediaFormat {
                has_audio: Some(has_audio),
                label,
                ..format
            }
        })
        .collect();
    MediaInfo { formats, ..info }
}

fn unavailable(message: &str) -> PawketError {
    PawketError::SourceUnavailable(message.to_owned())
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Post {
    #[serde(rename = "__typename")]
    typename: Option<String>,
    text: Option<String>,
    user: Option<User>,
    /// Kept raw so one malformed entry can be skipped individually.
    #[serde(rename = "mediaDetails", default)]
    media_details: Option<Vec<serde_json::Value>>,
    tombstone: Option<Tombstone>,
}

#[derive(Debug, Deserialize)]
struct User {
    screen_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Tombstone {
    text: Option<TombstoneText>,
}

#[derive(Debug, Deserialize)]
struct TombstoneText {
    text: String,
}

#[derive(Debug, Deserialize)]
struct Media {
    #[serde(rename = "type")]
    kind: String,
    media_url_https: Option<String>,
    original_info: Option<Size>,
    video_info: Option<VideoInfo>,
    ext_media_availability: Option<Availability>,
}

#[derive(Debug, Deserialize)]
struct Size {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
struct VideoInfo {
    duration_millis: Option<u64>,
    #[serde(default)]
    variants: Vec<Variant>,
}

#[derive(Debug, Deserialize)]
struct Variant {
    bitrate: Option<u64>,
    content_type: Option<String>,
    url: Option<String>,
}

impl Variant {
    fn has_type(&self, content_type: &str) -> bool {
        self.content_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case(content_type))
    }

    fn is_hls(&self) -> bool {
        self.has_type("application/x-mpegURL")
    }
}

#[derive(Debug, Deserialize)]
struct Availability {
    status: Option<String>,
}

/// Turn the embed JSON into Pawket's media description.
pub(crate) fn parse_post(source: &Url, post_id: &str, post: Post) -> Result<MediaInfo> {
    match post.typename.as_deref() {
        Some("Tweet") => {}
        Some("TweetTombstone") => {
            let reason = post
                .tombstone
                .and_then(|t| t.text)
                .map(|t| t.text.trim_end_matches("Learn more").trim().to_owned())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "It may be protected, age-restricted or removed.".into());
            return Err(PawketError::SourceUnavailable(format!(
                "X only shows this post to people who are signed in. {}",
                truncate(&reason, 200)
            )));
        }
        _ => return Err(unavailable("That X post isn’t available.")),
    }
    let screen_name = post
        .user
        .as_ref()
        .and_then(|u| u.screen_name.as_deref())
        .filter(|name| is_valid_screen_name(name));
    let mut formats = Vec::new();
    let mut unsupported = Vec::new();
    for (number, media) in decode_media(&post) {
        let (mut found, mut skipped) = match media {
            Some(media) => media_formats(number, &media),
            None => (Vec::new(), vec![unreadable(number)]),
        };
        formats.append(&mut found);
        unsupported.append(&mut skipped);
    }
    if formats.is_empty() {
        let message = if unsupported.is_empty() {
            "That X post has no video, GIF or photo to save.".to_owned()
        } else {
            let reasons: Vec<&str> = unsupported.iter().map(|u| u.reason.as_str()).collect();
            format!(
                "Pawket can’t save anything from that X post. {}",
                reasons.join(" ")
            )
        };
        return Err(PawketError::SourceUnavailable(message));
    }
    let canonical = match screen_name {
        Some(name) => format!("https://x.com/{name}/status/{post_id}"),
        None => source.to_string(),
    };
    Ok(MediaInfo {
        source_url: canonical,
        title: post_title(screen_name, post.text.as_deref(), post_id),
        formats: numbered(formats),
        provider: "x".into(),
        unsupported,
    })
}

fn media_formats(number: usize, media: &Media) -> (Vec<MediaFormat>, Vec<UnsupportedStream>) {
    let available = media
        .ext_media_availability
        .as_ref()
        .and_then(|a| a.status.as_deref())
        .is_none_or(|status| status.eq_ignore_ascii_case("available"));
    if !available {
        let reason = format!("File {number} was removed or withheld by X.");
        return (Vec::new(), vec![skipped("unavailable", reason)]);
    }
    match media.kind.as_str() {
        "photo" => match photo_format(number, media) {
            Some(format) => (vec![format], Vec::new()),
            None => (Vec::new(), vec![unreadable(number)]),
        },
        "video" | "animated_gif" => video_formats(number, media),
        other => {
            let kind: String = other.chars().take(MAX_KIND_CHARS).collect();
            let reason = format!("File {number} is a type Pawket doesn’t know yet ({kind}).");
            (Vec::new(), vec![skipped(&kind, reason)])
        }
    }
}

fn skipped(kind: &str, reason: String) -> UnsupportedStream {
    UnsupportedStream {
        kind: kind.to_owned(),
        reason,
    }
}

fn unreadable(number: usize) -> UnsupportedStream {
    skipped("unreadable", format!("Pawket couldn’t read file {number}."))
}

/// X handles are 1–15 ASCII letters, digits or underscores.
fn is_valid_screen_name(name: &str) -> bool {
    (1..=15).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn photo_format(number: usize, media: &Media) -> Option<MediaFormat> {
    let mut url = media_url(media.media_url_https.as_deref())?;
    let extension = url.path().rsplit('.').next()?.to_ascii_lowercase();
    let (mime, extension) = match extension.as_str() {
        "jpg" | "jpeg" => ("image/jpeg", "jpg"),
        "png" => ("image/png", "png"),
        "webp" => ("image/webp", "webp"),
        _ => return None,
    };
    // `name=orig` asks pbs.twimg.com for the originally uploaded resolution.
    url.set_query(Some("name=orig"));
    let size = media.original_info.as_ref();
    let dimensions = size
        .map(|s| format!(" · {}×{}", s.width, s.height))
        .unwrap_or_default();
    Some(MediaFormat {
        id: format!("x-{number}-photo"),
        url: url.to_string(),
        extension: extension.into(),
        mime_type: mime.into(),
        kind: MediaKind::Image,
        item: u32::try_from(number).ok(),
        label: Some(format!("Photo{dimensions}")),
        width: size.map(|s| s.width),
        height: size.map(|s| s.height),
        ..MediaFormat::default()
    })
}

fn video_formats(number: usize, media: &Media) -> (Vec<MediaFormat>, Vec<UnsupportedStream>) {
    let is_gif = media.kind == "animated_gif";
    let Some(info) = media.video_info.as_ref() else {
        return (Vec::new(), vec![unreadable(number)]);
    };
    let mut mp4: Vec<&Variant> = info
        .variants
        .iter()
        .filter(|v| v.has_type("video/mp4"))
        .collect();
    mp4.sort_by_key(|v| std::cmp::Reverse(v.bitrate.unwrap_or(0)));
    // Only a lone rendition can safely borrow the upload's dimensions;
    // with several, it would mislabel them and collapse their IDs.
    let is_single = mp4.len() == 1;
    let has_hls = info.variants.iter().any(Variant::is_hls);
    let mut formats: Vec<MediaFormat> = Vec::new();
    for variant in mp4 {
        let Some(url) = media_url(variant.url.as_deref()) else {
            continue;
        };
        let (width, height) = dimensions_from_url(&url)
            .or_else(|| {
                let size = media.original_info.as_ref().filter(|_| is_single)?;
                Some((size.width, size.height))
            })
            .unzip();
        let id = match (is_gif, width, height) {
            (true, _, _) => format!("x-{number}-gif"),
            (false, Some(w), Some(h)) => format!("x-{number}-{w}x{h}"),
            _ => format!("x-{number}-{}", variant.bitrate.unwrap_or(0)),
        };
        if formats.iter().any(|f| f.id == id) {
            continue;
        }
        formats.push(MediaFormat {
            id,
            url: url.to_string(),
            extension: "mp4".into(),
            mime_type: "video/mp4".into(),
            kind: MediaKind::Video,
            label: Some(video_label(is_gif, width.zip(height))),
            width,
            height,
            bitrate: variant.bitrate.filter(|b| *b > 0),
            duration_ms: info.duration_millis.filter(|d| *d > 0),
            // X converts GIF uploads to silent MP4s. For ordinary videos the
            // embed data does not say whether there is an audio track.
            has_audio: is_gif.then_some(false),
            item: u32::try_from(number).ok(),
            ..MediaFormat::default()
        });
    }
    let mut unsupported = Vec::new();
    if has_hls {
        let reason = if formats.is_empty() {
            format!("Video {number} is only available as a stream, which Pawket can’t save yet.")
        } else {
            format!("Video {number} also comes as a stream. You don’t need it: the MP4 files are the same video.")
        };
        unsupported.push(skipped("hls", reason));
    } else if formats.is_empty() {
        unsupported.push(unreadable(number));
    }
    (formats, unsupported)
}

/// A short label for one rendition: its size, e.g. "1280×720".
/// Bitrate stays in the `bitrate` field rather than the label.
fn video_label(is_gif: bool, size: Option<(u32, u32)>) -> String {
    let size = size.map(|(w, h)| format!("{w}×{h}"));
    match (is_gif, size) {
        (true, Some(size)) => format!("GIF · {size} · {NO_AUDIO}"),
        (true, None) => format!("GIF · {NO_AUDIO}"),
        (false, Some(size)) => size,
        (false, None) => "Video".to_owned(),
    }
}

/// In a post with several photos or videos, say which one each file is:
/// "Photo 2 · 1386×1415", "Video 1 · 1280×720". A single item needs no number.
fn numbered(formats: Vec<MediaFormat>) -> Vec<MediaFormat> {
    let mut items: Vec<u32> = formats.iter().filter_map(|f| f.item).collect();
    items.dedup();
    if items.len() < 2 {
        return formats;
    }
    formats
        .into_iter()
        .map(|format| {
            let (Some(item), Some(label)) = (format.item, format.label.clone()) else {
                return format;
            };
            let label = match label.split_once(" · ") {
                Some((kind @ ("Photo" | "GIF"), rest)) => format!("{kind} {item} · {rest}"),
                _ if label == "Photo" || label == "GIF" => format!("{label} {item}"),
                _ => format!("Video {item} · {label}"),
            };
            MediaFormat {
                label: Some(label),
                ..format
            }
        })
        .collect()
}

/// X encodes rendition sizes in the path, e.g. `/vid/avc1/1280x720/name.mp4`.
fn dimensions_from_url(url: &Url) -> Option<(u32, u32)> {
    url.path_segments()?.find_map(|segment| {
        let (w, h) = segment.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    })
}

fn post_title(screen_name: Option<&str>, text: Option<&str>, post_id: &str) -> String {
    // Drop the trailing t.co links X appends for attached media.
    let words: Vec<&str> = text
        .unwrap_or("")
        .split_whitespace()
        .filter(|word| !word.starts_with("https://t.co/"))
        .collect();
    let body = truncate(&decode_entities(&words.join(" ")), MAX_TITLE_CHARS);
    match (screen_name, body.is_empty()) {
        (Some(name), false) => format!("@{name} – {body}"),
        (Some(name), true) => format!("@{name} – X post {post_id}"),
        (None, false) => body,
        (None, true) => format!("X post {post_id}"),
    }
}

/// X post text arrives with the HTML escapes `&amp;`, `&lt;` and `&gt;`.
fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests;
