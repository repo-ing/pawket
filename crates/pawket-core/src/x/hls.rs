//! Reads an HLS master playlist only to learn whether a video has sound.

const AUDIO_CODECS: [&str; 5] = ["mp4a", "ac-3", "ec-3", "opus", "flac"];

/// `Some(true)` when the master playlist declares an audio rendition or an
/// audio codec; `Some(false)` only when every variant lists its codecs and
/// none is audio; otherwise `None` (unknown).
pub(super) fn playlist_has_audio(playlist: &str) -> Option<bool> {
    if !playlist.trim_start().starts_with("#EXTM3U") {
        return None;
    }
    let declares_audio_rendition = playlist
        .lines()
        .filter_map(|line| line.strip_prefix("#EXT-X-MEDIA:"))
        .any(|attributes| hls_attribute(attributes, "TYPE") == Some("AUDIO"));
    if declares_audio_rendition {
        return Some(true);
    }
    let codec_lists: Vec<Option<&str>> = playlist
        .lines()
        .filter_map(|line| line.strip_prefix("#EXT-X-STREAM-INF:"))
        .map(|attributes| hls_attribute(attributes, "CODECS"))
        .collect();
    if codec_lists.is_empty() || codec_lists.iter().any(Option::is_none) {
        return None;
    }
    let is_audio_codec = |codec: &str| {
        let codec = codec.trim().to_ascii_lowercase();
        AUDIO_CODECS.iter().any(|prefix| codec.starts_with(prefix))
    };
    Some(
        codec_lists
            .iter()
            .flatten()
            .any(|list| list.split(',').any(is_audio_codec)),
    )
}

/// Value of `name` in an HLS attribute list (`A=1,B="x,y",C=z`), unquoted.
pub(super) fn hls_attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attributes;
    while !rest.is_empty() {
        let (key, after_key) = rest.split_once('=')?;
        let (value, after_value) = match after_key.strip_prefix('"') {
            Some(quoted) => {
                let (value, tail) = quoted.split_once('"')?;
                (value, tail.strip_prefix(',').unwrap_or(tail))
            }
            None => after_key.split_once(',').unwrap_or((after_key, "")),
        };
        if key.trim() == name {
            return Some(value);
        }
        rest = after_value;
    }
    None
}
