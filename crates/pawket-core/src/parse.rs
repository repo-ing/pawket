use crate::{network, MediaFormat, MediaInfo, MediaKind, PawketError, Result};
use scraper::{Html, Selector};
use std::collections::HashSet;
use url::Url;

pub fn title_from_url(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .unwrap_or("media")
        .to_string()
}

fn type_by_mime(mime: &str) -> Option<(MediaKind, &'static str)> {
    Some(match mime {
        "video/mp4" => (MediaKind::Video, "mp4"),
        "video/webm" => (MediaKind::Video, "webm"),
        "video/quicktime" => (MediaKind::Video, "mov"),
        "video/x-m4v" => (MediaKind::Video, "m4v"),
        "video/ogg" => (MediaKind::Video, "ogv"),
        "audio/mpeg" | "audio/mp3" => (MediaKind::Audio, "mp3"),
        "audio/mp4" | "audio/x-m4a" => (MediaKind::Audio, "m4a"),
        "audio/aac" => (MediaKind::Audio, "aac"),
        "audio/ogg" => (MediaKind::Audio, "ogg"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => (MediaKind::Audio, "wav"),
        "audio/flac" | "audio/x-flac" => (MediaKind::Audio, "flac"),
        "audio/webm" => (MediaKind::Audio, "webm"),
        "image/jpeg" => (MediaKind::Image, "jpg"),
        "image/png" => (MediaKind::Image, "png"),
        "image/webp" => (MediaKind::Image, "webp"),
        "image/gif" => (MediaKind::Image, "gif"),
        "image/avif" => (MediaKind::Image, "avif"),
        _ => return None,
    })
}

fn mime_by_extension(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "m4v" => "video/x-m4v",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        _ => return None,
    })
}

pub fn direct_format(url: &Url, mime: &str, size_bytes: Option<u64>) -> Option<MediaFormat> {
    let extension = url
        .path()
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let effective_mime =
        if mime.is_empty() || mime == "application/octet-stream" || mime == "binary/octet-stream" {
            mime_by_extension(&extension)?
        } else {
            mime
        };
    let (kind, extension) = type_by_mime(effective_mime)?;
    Some(MediaFormat {
        id: "original".into(),
        url: url.to_string(),
        extension: extension.into(),
        mime_type: effective_mime.into(),
        kind,
        size_bytes,
        ..MediaFormat::default()
    })
}

const MAX_LABEL_CHARS: usize = 60;

/// Descriptions a page states for a `<source>`, such as MediaWiki's
/// `data-shorttitle="VP9 240P"` and `data-height`. Never inferred.
#[derive(Default)]
struct SourceHints {
    label: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

impl SourceHints {
    fn from_element(element: &scraper::ElementRef) -> Self {
        let attr = |name: &str| element.value().attr(name);
        let label = ["data-shorttitle", "label", "data-title", "title"]
            .iter()
            .find_map(|name| attr(name))
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| !text.is_empty())
            .map(|text| text.chars().take(MAX_LABEL_CHARS).collect());
        let number = |name: &str| {
            attr(name)
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|n| *n > 0)
        };
        Self {
            label,
            width: number("data-width"),
            height: number("data-height"),
        }
    }
}

pub fn parse_html(base: &Url, source: &str) -> Result<MediaInfo> {
    let html = Html::parse_document(source);
    let title_selector = Selector::parse("title").expect("constant selector");
    let meta_selector = Selector::parse("meta[property], meta[name]").expect("constant selector");
    let media_selector =
        Selector::parse("video[src], audio[src], video source[src], audio source[src]")
            .expect("constant selector");
    let mut title = html
        .select(&title_selector)
        .next()
        .map(|element| element.text().collect::<String>())
        .unwrap_or_else(|| title_from_url(base));
    let mut candidates: Vec<(String, String, Option<MediaKind>, SourceHints)> = Vec::new();
    for element in html.select(&meta_selector) {
        let property = element
            .value()
            .attr("property")
            .or_else(|| element.value().attr("name"))
            .unwrap_or("");
        let content = element.value().attr("content").unwrap_or("");
        if property == "og:title" && !content.trim().is_empty() {
            title = content.into();
        }
        let kind = match property {
            "og:video" | "og:video:url" | "og:video:secure_url" => Some(MediaKind::Video),
            "og:audio" | "og:audio:url" | "og:audio:secure_url" => Some(MediaKind::Audio),
            // Images advertised as og:image are commonly decorative preview
            // thumbnails; they are not silently substituted for a video.
            _ => None,
        };
        if kind.is_some() {
            candidates.push((content.into(), String::new(), kind, SourceHints::default()));
        }
    }
    for element in html.select(&media_selector) {
        let src = element.value().attr("src").unwrap_or("");
        let mime = element
            .value()
            .attr("type")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let kind = match element.value().name() {
            "audio" => Some(MediaKind::Audio),
            "video" => Some(MediaKind::Video),
            _ => element
                .parent()
                .and_then(scraper::ElementRef::wrap)
                .and_then(|p| match p.value().name() {
                    "audio" => Some(MediaKind::Audio),
                    "video" => Some(MediaKind::Video),
                    _ => None,
                }),
        };
        candidates.push((
            src.into(),
            mime.into(),
            kind,
            SourceHints::from_element(&element),
        ));
    }
    let mut seen = HashSet::new();
    let mut formats = Vec::new();
    for (src, mime, hinted_kind, hints) in candidates {
        if formats.len() >= 32 {
            break;
        }
        if src.trim().is_empty() {
            continue;
        }
        let Ok(url) = base.join(src.trim()) else {
            continue;
        };
        let Ok(url) = network::validate_url(url.as_str()) else {
            continue;
        };
        if !seen.insert(url.to_string()) {
            continue;
        }
        let mut format = match direct_format(&url, &mime, None) {
            Some(format) => format,
            None => continue,
        };
        if let Some(kind) = hinted_kind {
            // Declared audio sources with a webm extension should be presented
            // as audio; no conversion is implied or performed.
            if kind == MediaKind::Audio && format.mime_type == "video/webm" && mime.is_empty() {
                format.kind = kind;
                format.mime_type = "audio/webm".into();
            }
            if format.kind != kind {
                continue;
            }
        }
        format.id = format!("source-{}", formats.len() + 1);
        format.label = hints.label;
        format.width = hints.width;
        format.height = hints.height;
        formats.push(format);
    }
    if formats.is_empty() {
        return Err(PawketError::Unsupported);
    }
    title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        title = "media".into();
    }
    title = title.chars().take(200).collect();
    Ok(MediaInfo {
        source_url: base.to_string(),
        title,
        formats,
        provider: "html_media".into(),
        unsupported: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_media_and_deduplicates() {
        let source = r#"<title> A video </title><meta property="og:title" content="Clip &amp; Music">
            <meta property="og:video" content="https://cdn.example.com/clip.mp4">
            <video><source src="https://cdn.example.com/clip.mp4" type="video/mp4">
            <source src="/clip.webm" type="video/webm"></video>
            <audio src="/track.mp3"></audio>"#;
        let info = parse_html(&Url::parse("https://example.com/post/1").unwrap(), source).unwrap();
        assert_eq!(info.title, "Clip & Music");
        assert_eq!(info.formats.len(), 3);
        assert_eq!(info.formats[1].url, "https://example.com/clip.webm");
        assert_eq!(info.formats[2].kind, MediaKind::Audio);
        assert_eq!(info.formats[0].id, "source-1");
        assert_eq!(info.provider, "html_media");
    }

    #[test]
    fn keeps_quality_labels_the_page_states() {
        let source = r#"<video>
            <source src="/a.240p.webm" type="video/webm" data-shorttitle="VP9 240P" data-width="426" data-height="240">
            <source src="/a.1080p.webm" type="video/webm" data-height="nope"></video>"#;
        let info = parse_html(&Url::parse("https://example.com/a").unwrap(), source).unwrap();
        assert_eq!(info.formats[0].label.as_deref(), Some("VP9 240P"));
        assert_eq!(
            (info.formats[0].width, info.formats[0].height),
            (Some(426), Some(240))
        );
        assert_eq!(info.formats[1].label, None);
        assert_eq!(info.formats[1].height, None);
    }

    #[test]
    fn refuses_private_embeds_and_does_not_substitute_thumbnail() {
        let source = r#"<meta property="og:image" content="https://cdn.example.com/image.jpg">
            <video src="https://127.0.0.1/a.mp4"></video>
            <video src="http://example.com/a.mp4"></video>
            <video src="blob:abc"></video><video src="/stream.m3u8"></video>"#;
        assert!(matches!(
            parse_html(&Url::parse("https://example.com/a").unwrap(), source),
            Err(PawketError::Unsupported)
        ));
    }

    #[test]
    fn html_content_type_cannot_masquerade_as_mp4() {
        let url = Url::parse("https://example.com/a.mp4").unwrap();
        assert!(direct_format(&url, "text/html", None).is_none());
        assert!(direct_format(&url, "application/octet-stream", Some(20)).is_some());
    }

    #[test]
    fn mime_type_supports_extensionless_media_and_preserves_kind() {
        let url = Url::parse("https://cdn.example.com/download?id=1").unwrap();
        let format = direct_format(&url, "audio/mp4", None).unwrap();
        assert_eq!(format.kind, MediaKind::Audio);
        assert_eq!(format.extension, "m4a");
        assert!(direct_format(&url, "application/octet-stream", None).is_none());
    }

    #[test]
    fn does_not_invent_conversion_formats() {
        let source = "<video src='/original.mp4'></video>";
        let info = parse_html(&Url::parse("https://example.com/a").unwrap(), source).unwrap();
        assert_eq!(info.formats.len(), 1);
        assert_eq!(info.formats[0].extension, "mp4");
        assert_eq!(info.formats[0].kind, MediaKind::Video);
    }
}
