//! Preview adapter for direct, clear media URLs in YouTube's public watch page.

use crate::{
    network, parse, providers::host_matches, Conversion, MediaFormat, MediaInfo, MediaKind,
    PawketError, Result, UnsupportedStream,
};
use scraper::{Html, Selector};
use serde::Deserialize;
use std::collections::HashSet;
use url::Url;

const MAX_WATCH_BYTES: u64 = 4 * 1024 * 1024;

fn watch_host(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| host_matches(host, "youtube.com"))
}

pub(crate) fn video_id(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let mut path = url.path_segments()?;
    let first = path.next()?;
    let id = if host.eq_ignore_ascii_case("youtu.be") {
        first.into()
    } else if watch_host(url) || host_matches(host, "youtube-nocookie.com") {
        match first {
            "watch" => url.query_pairs().find(|(key, _)| key == "v")?.1,
            "shorts" | "live" | "embed" => path.next()?.into(),
            _ => return None,
        }
    } else {
        return None;
    };
    (id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)))
    .then(|| id.into_owned())
}

pub(crate) fn media_url_allowed(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| host_matches(host, "googlevideo.com"))
        && (url.path() == "/videoplayback" || url.path().starts_with("/videoplayback/"))
}

pub(crate) async fn inspect(source: &Url, id: &str, can_mux: bool) -> Result<MediaInfo> {
    let player = fetch_player(id).await?;
    parse_player(source, id, player, can_mux)
}

async fn fetch_player(id: &str) -> Result<Player> {
    let mut watch = Url::parse("https://www.youtube.com/watch").expect("constant watch URL");
    watch.query_pairs_mut().append_pair("v", id);
    let (_, response) = network::fetch_scoped(watch, watch_host).await?;
    let bytes = network::read_limited(response, MAX_WATCH_BYTES).await?;
    let page = String::from_utf8_lossy(&bytes);
    player_response(&page).ok_or_else(|| {
        unavailable("The YouTube watch page did not expose its public player response.")
    })
}

/// Read format metadata from the public player. The backend still resolves
/// the selected stream at save time, using its ordinary format selector.
pub(crate) async fn inspect_catalog(
    source: &Url,
    id: &str,
    abilities: crate::ytdlp::Abilities,
) -> Option<MediaInfo> {
    catalog(source, id, abilities, fetch_player(id).await.ok()?)
}

fn catalog(
    source: &Url,
    id: &str,
    abilities: crate::ytdlp::Abilities,
    player: Player,
) -> Option<MediaInfo> {
    let details = player.video_details?;
    let streams = player.streaming_data?;
    if player.playability_status.status != "OK"
        || details.video_id != id
        || details.is_live
        || !streams.license_infos.is_empty()
    {
        return None;
    }
    let mut tracks = Vec::new();
    for (combined, entries) in [(true, streams.formats), (false, streams.adaptive_formats)] {
        for value in entries.into_iter().take(128) {
            let Ok(format) = Format::deserialize(value) else {
                continue;
            };
            if !format.drm_families.is_empty() {
                continue;
            }
            let mime = format.mime_type.split(';').next()?;
            let (kind, extension) = mime.split_once('/')?;
            let audio = kind == "audio";
            if !audio && kind != "video" {
                continue;
            }
            tracks.push(crate::ytdlp::Track {
                height: format.height,
                has_video: !audio,
                audio_only: audio,
                has_audio: Some(audio || combined),
                h264: format.mime_type.contains("avc1"),
                extension: Some(
                    if audio && extension == "mp4" {
                        "m4a"
                    } else {
                        extension
                    }
                    .into(),
                ),
                size: format.content_length.and_then(|n| n.parse().ok()),
                audio_rate: format.bitrate.unwrap_or_default() as f64 / 1000.0,
            });
        }
    }
    if !tracks
        .iter()
        .any(|t| t.has_video && (abilities.merge || t.has_audio == Some(true)))
        || !tracks.iter().any(|t| t.has_audio == Some(true))
    {
        return None;
    }
    let base = MediaFormat {
        url: source.to_string(),
        duration_ms: details
            .length_seconds?
            .parse::<u64>()
            .ok()?
            .checked_mul(1000),
        ..Default::default()
    };
    let formats = crate::ytdlp::single(&base, &tracks, abilities);
    Some(MediaInfo {
        title: details.title?,
        source_url: source.to_string(),
        provider: crate::ytdlp::PROVIDER.into(),
        formats,
        unsupported: Vec::new(),
    })
}

fn unavailable(message: &str) -> PawketError {
    PawketError::SourceUnavailable(message.into())
}

fn player_response(page: &str) -> Option<Player> {
    let document = Html::parse_document(page);
    let selector = Selector::parse("script").expect("constant selector");
    for script in document
        .select(&selector)
        .flat_map(|element| element.text())
    {
        for prefix in [
            "var ytInitialPlayerResponse",
            "window[\"ytInitialPlayerResponse\"]",
            "window['ytInitialPlayerResponse']",
            "window.ytInitialPlayerResponse",
        ] {
            if let Some((_, value)) = script.split_once(prefix) {
                if let Some(json) = value.trim_start().strip_prefix('=') {
                    if let Some(Ok(player)) = serde_json::Deserializer::from_str(json)
                        .into_iter::<Player>()
                        .next()
                    {
                        return Some(player);
                    }
                }
            }
        }
    }
    None
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Player {
    playability_status: Playability,
    video_details: Option<Details>,
    streaming_data: Option<StreamingData>,
}

#[derive(Deserialize)]
struct Playability {
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Details {
    video_id: String,
    length_seconds: Option<String>,
    title: Option<String>,
    #[serde(default)]
    is_live: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamingData {
    #[serde(default)]
    formats: Vec<serde_json::Value>,
    #[serde(default)]
    adaptive_formats: Vec<serde_json::Value>,
    dash_manifest_url: Option<String>,
    hls_manifest_url: Option<String>,
    #[serde(default)]
    license_infos: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Format {
    itag: u32,
    url: Option<String>,
    mime_type: String,
    quality_label: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    bitrate: Option<u64>,
    content_length: Option<String>,
    #[serde(default)]
    drm_families: Vec<serde_json::Value>,
}

fn parse_player(source: &Url, id: &str, player: Player, can_mux: bool) -> Result<MediaInfo> {
    if player.playability_status.status != "OK" {
        return Err(unavailable(
            "This video isn't available through YouTube's public player.",
        ));
    }
    let details = player
        .video_details
        .filter(|details| details.video_id == id && !details.is_live)
        .ok_or_else(|| {
            unavailable(
                "The public YouTube player didn't identify an on-demand video at this link.",
            )
        })?;
    let data = player
        .streaming_data
        .filter(|data| data.license_infos.is_empty())
        .ok_or_else(|| {
            unavailable("The public YouTube player didn't expose clear video formats.")
        })?;

    let mut unsupported = Vec::new();
    for (kind, manifest) in [
        ("hls", data.hls_manifest_url),
        ("dash", data.dash_manifest_url),
    ] {
        if manifest.is_some() {
            unsupported.push(UnsupportedStream {
                kind: kind.into(),
                reason: "This preview saves direct media files only.".into(),
            });
        }
    }

    let mut seen = HashSet::new();
    let mut muxed = Vec::new();
    let mut video = Vec::new();
    let mut audio = Vec::new();
    let mut skipped = false;
    for (combined, entries) in [(true, data.formats), (false, data.adaptive_formats)] {
        for entry in entries.into_iter().take(128) {
            let choice = Format::deserialize(entry)
                .ok()
                .and_then(|entry| to_format(entry, combined));
            let Some(format) = choice else {
                skipped = true;
                continue;
            };
            if !seen.insert(format.id.clone()) {
                continue;
            }
            match (format.kind, combined) {
                (MediaKind::Video, true) => muxed.push(format),
                (MediaKind::Video, false) => video.push(format),
                (MediaKind::Audio, _) => audio.push(format),
                _ => {}
            }
        }
    }
    if skipped {
        unsupported.push(UnsupportedStream {
            kind: "youtube_formats".into(),
            reason: "Some formats lack a usable direct URL, require a player challenge, or exceed Pawket's limits.".into(),
        });
    }
    let sort_video = |formats: &mut Vec<MediaFormat>| {
        formats.sort_by_key(|format| {
            std::cmp::Reverse((format.height, format.extension == "mp4", format.bitrate))
        });
    };
    sort_video(&mut muxed);
    sort_video(&mut video);
    audio.sort_by_key(|format| std::cmp::Reverse(format.bitrate));
    let with_sound = if can_mux {
        joinable_video(&video, &audio)
    } else {
        Vec::new()
    };
    let formats: Vec<_> = muxed
        .into_iter()
        .take(8)
        .chain(video.into_iter().take(8))
        .chain(audio.into_iter().take(4))
        .chain(with_sound)
        .collect();
    if formats.is_empty() {
        return Err(unavailable("YouTube didn't expose a usable direct media file. This video may require a player challenge or a stream format outside the preview."));
    }

    let title: String = details
        .title
        .unwrap_or_default()
        .trim()
        .chars()
        .take(200)
        .collect();
    Ok(MediaInfo {
        source_url: source.to_string(),
        title: if title.is_empty() {
            format!("YouTube {id}")
        } else {
            title
        },
        formats,
        provider: "youtube".into(),
        unsupported,
    })
}

fn joinable_video(video: &[MediaFormat], audio: &[MediaFormat]) -> Vec<MediaFormat> {
    let Some(audio) = audio.iter().find(|track| track.extension == "m4a") else {
        return Vec::new();
    };
    video
        .iter()
        .filter(|track| track.extension == "mp4")
        .take(8)
        .map(|video| {
            let label = video.label.as_deref().unwrap_or("Video");
            let label = label.strip_suffix(" · no sound").unwrap_or(label);
            MediaFormat {
                id: format!("{}-with-{}", video.id, audio.id),
                label: Some(format!("{label} · with sound")),
                has_audio: Some(true),
                size_bytes: None,
                bitrate: None,
                conversion: Some(Conversion::Mp4),
                audio_source_url: Some(audio.url.clone()),
                ..video.clone()
            }
        })
        .collect()
}

fn to_format(entry: Format, combined: bool) -> Option<MediaFormat> {
    if !entry.drm_families.is_empty() {
        return None;
    }
    let url = network::validate_url(entry.url.as_deref()?).ok()?;
    if !media_url_allowed(&url) {
        return None;
    }
    let mime = entry.mime_type.split(';').next()?.trim();
    let size = entry
        .content_length
        .as_deref()
        .and_then(|size| size.parse().ok());
    let mut format = parse::direct_format(&url, mime, size)?;
    if format.kind == MediaKind::Image {
        return None;
    }
    format.id = format!("youtube-{}", entry.itag);
    format.width = entry.width;
    format.height = entry.height;
    format.bitrate = entry.bitrate;
    format.has_audio = Some(combined || format.kind == MediaKind::Audio);
    format.label = match format.kind {
        MediaKind::Video => {
            let quality = entry
                .quality_label
                .filter(|quality| !quality.trim().is_empty())
                .map(|quality| quality.chars().take(40).collect())
                .or_else(|| entry.height.map(|height| format!("{height}p")))
                .unwrap_or_else(|| "Video".into());
            let audio = if combined { "" } else { " · no sound" };
            Some(format!("{quality}{audio}"))
        }
        MediaKind::Audio => entry
            .bitrate
            .map(|rate| format!("Audio · {} kbps", rate / 1000)),
        MediaKind::Image => None,
    };
    Some(format)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "jNQXAC9IVRw";

    #[test]
    fn recognises_video_links_and_rejects_lookalike_hosts_and_playlists() {
        for url in [
            "https://www.youtube.com/watch?v=jNQXAC9IVRw&list=anything",
            "https://m.youtube.com/shorts/jNQXAC9IVRw",
            "https://youtu.be/jNQXAC9IVRw?t=7",
            "https://youtu.be/jNQXAC9IVRw/",
            "https://www.youtube-nocookie.com/embed/jNQXAC9IVRw",
        ] {
            assert_eq!(video_id(&Url::parse(url).unwrap()).as_deref(), Some(ID));
        }
        for url in [
            "https://www.youtube.com/playlist?list=jNQXAC9IVRw",
            "https://www.youtube.com.evil.org/watch?v=jNQXAC9IVRw",
            "https://www.youtube.com/watch?v=bad",
        ] {
            assert_eq!(video_id(&Url::parse(url).unwrap()), None);
        }
    }

    #[test]
    fn extracts_only_clear_pinned_direct_formats_from_player_json() {
        let source = Url::parse("https://www.youtube.com/watch?v=jNQXAC9IVRw").unwrap();
        let html = r#"<script>var ytInitialPlayerResponse = {
            "playabilityStatus":{"status":"OK"},
            "videoDetails":{"videoId":"jNQXAC9IVRw","title":"Fixture"},
            "streamingData":{
                "hlsManifestUrl":"https://manifest.googlevideo.com/hls",
                "formats":[
                    {"itag":99,"mimeType":"video/mp4","signatureCipher":"unused"},
                    {"itag":98,"mimeType":"video/mp4","url":"https://r.googlevideo.com.evil.org/videoplayback"},
                    {"itag":97,"mimeType":"video/mp4","url":"https://r.googlevideo.com/videoplayback","drmFamilies":["WIDEVINE"]},
                    {"itag":96,"mimeType":"video/mp4","url":"https://r.googlevideo.com/other"},
                    {"itag":18,"mimeType":"video/mp4; codecs=\"avc1, mp4a\"","url":"https://r.googlevideo.com/videoplayback?mime=video%2Fmp4&sig=test","height":360}
                ],
                "adaptiveFormats":[
                    {"itag":137,"mimeType":"video/mp4","url":"https://r.googlevideo.com/videoplayback","height":1080},
                    {"itag":140,"mimeType":"audio/mp4","url":"https://r.googlevideo.com/videoplayback","bitrate":128000}
                ]
            }};</script>"#;
        let mut info = parse_player(&source, ID, player_response(html).unwrap(), true).unwrap();
        assert_eq!(info.title, "Fixture");
        assert_eq!(
            info.formats
                .iter()
                .map(|f| f.id.as_str())
                .collect::<Vec<_>>(),
            [
                "youtube-18",
                "youtube-137",
                "youtube-140",
                "youtube-137-with-youtube-140"
            ]
        );
        assert_eq!(info.formats[0].has_audio, Some(true));
        assert_eq!(info.formats[1].has_audio, Some(false));
        assert_eq!(info.formats[2].extension, "m4a");
        assert_eq!(info.unsupported.len(), 2);
        assert_eq!(info.formats[3].has_audio, Some(true));
        assert_eq!(info.formats[3].conversion, Some(Conversion::Mp4));
        assert_eq!(
            info.formats[3].audio_source_url.as_deref(),
            Some(info.formats[2].url.as_str())
        );
        crate::convert::add_formats(&mut info, crate::convert::Encoders::default());
        assert!(info.formats.iter().any(|f| f.id == "youtube-140-as-mp3"));
        assert!(!info.formats.iter().any(|f| f.id == "youtube-137-as-mp3"));
    }

    #[test]
    fn catalog_uses_track_metadata_and_defers_stream_resolution() {
        let source = Url::parse("https://www.youtube.com/watch?v=jNQXAC9IVRw").unwrap();
        let page = r#"<script>var ytInitialPlayerResponse = {"playabilityStatus":{"status":"OK"},
            "videoDetails":{"videoId":"jNQXAC9IVRw","title":"Sample","lengthSeconds":"42"},
            "streamingData":{"adaptiveFormats":[
                {"itag":137,"mimeType":"video/mp4","height":1080},
                {"itag":140,"mimeType":"audio/mp4","bitrate":128000}]}};</script>"#;
        let abilities = crate::ytdlp::Abilities {
            merge: true,
            opus: false,
        };
        let parse = |html: &str| catalog(&source, ID, abilities, player_response(html).unwrap());
        let info = parse(page).unwrap();
        assert!(info.formats.iter().any(|f| f.id == "ytdlp-1080p"));
        assert!(info
            .formats
            .iter()
            .all(|f| f.url == source.as_str() && f.duration_ms == Some(42000)));
        assert!(parse(&page.replace("OK", "LOGIN_REQUIRED")).is_none());
        assert!(parse(&page.replace(ID, "another-id")).is_none());
        assert!(parse(&page.replace("adaptiveFormats", "unknown")).is_none());
    }

    #[tokio::test]
    #[ignore = "requires direct HTTPS access to YouTube and Google Video"]
    async fn downloads_a_public_youtube_mp4() {
        let link = std::env::var("PAWKET_TEST_YOUTUBE_URL")
            .unwrap_or_else(|_| format!("https://www.youtube.com/watch?v={ID}"));
        let engine = crate::Engine::new().unwrap().with_youtube_preview(true);
        let info = engine.inspect(&link).await.unwrap();
        let format = info
            .formats
            .iter()
            .find(|format| format.extension == "mp4" && format.has_audio == Some(true))
            .expect("the test video should expose a combined MP4");
        let directory = tempfile::tempdir().unwrap();
        let output = engine
            .download_format(&info, format, directory.path())
            .await
            .unwrap();
        assert!(output.filename.ends_with(".mp4"));
        let bytes = tokio::fs::read(output.path).await.unwrap();
        assert_eq!(bytes.len() as u64, output.bytes);
        assert_eq!(bytes.get(4..8), Some(&b"ftyp"[..]));
    }
}
