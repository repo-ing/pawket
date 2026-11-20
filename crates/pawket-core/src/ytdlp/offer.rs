//! Which choices Pawket offers for a yt-dlp result.
//!
//! Every video height comes with sound, without sound, and (when the site
//! has no H.264 at that height) re-encoded so it plays everywhere. Audio is
//! offered as the original track, MP3 at three bitrates, WAV, FLAC and Opus.

use crate::{
    choice::{AudioFormat, Choice},
    convert::{DEFAULT_MP3_KBPS, MP3_KBPS},
    Conversion, MediaFormat, MediaKind,
};

const MAX_HEIGHTS: usize = 6;
pub(crate) const ITEM_PREFIX: &str = "ytdlp-item-";

/// What the local tools can do with yt-dlp's downloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Abilities {
    /// FFmpeg is configured: tracks can be joined and converted.
    pub(crate) merge: bool,
    /// FFmpeg has the Opus encoder.
    pub(crate) opus: bool,
}

/// One stream yt-dlp listed, reduced to what offering needs.
#[derive(Clone, Debug, Default)]
pub(crate) struct Track {
    pub(crate) height: Option<u32>,
    pub(crate) has_video: bool,
    pub(crate) audio_only: bool,
    /// `None` when the site doesn't say.
    pub(crate) has_audio: Option<bool>,
    pub(crate) h264: bool,
    pub(crate) extension: Option<String>,
    pub(crate) size: Option<u64>,
    pub(crate) audio_rate: f64,
}

/// One public playlist entry.
pub(crate) struct Entry {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) duration_ms: Option<u64>,
}

fn mime_for(kind: MediaKind, ext: &str) -> String {
    match (kind, ext) {
        (_, "mp3") => "audio/mpeg".into(),
        (_, "m4a") => "audio/mp4".into(),
        (_, "opus") => "audio/ogg".into(),
        (MediaKind::Audio, other) => format!("audio/{other}"),
        (_, other) => format!("video/{other}"),
    }
}

fn video(
    base: &MediaFormat,
    id: String,
    label: String,
    extension: &str,
    choice: Choice,
) -> MediaFormat {
    MediaFormat {
        id,
        mime_type: mime_for(MediaKind::Video, extension),
        extension: extension.into(),
        kind: MediaKind::Video,
        label: Some(label),
        height: choice.height,
        has_audio: Some(choice.sound),
        conversion: None,
        choice: Some(choice),
        ..base.clone()
    }
}

fn muted(format: MediaFormat) -> MediaFormat {
    MediaFormat {
        conversion: Some(Conversion::Mute),
        has_audio: Some(false),
        ..format
    }
}

/// MP3 at each bitrate, WAV, FLAC and Opus, each as a conversion of the
/// best audio. `prefix` is `ytdlp-` or a playlist item's prefix.
fn audio_conversions(base: &MediaFormat, prefix: &str, abilities: Abilities) -> Vec<MediaFormat> {
    let convert = |suffix: String, conversion: Conversion, kbps: Option<u32>, label: String| {
        let extension = conversion.extension();
        MediaFormat {
            id: format!("{prefix}{suffix}"),
            mime_type: mime_for(MediaKind::Audio, extension),
            extension: extension.into(),
            kind: MediaKind::Audio,
            label: Some(label),
            has_audio: Some(true),
            bitrate: kbps.map(|kbps| u64::from(kbps) * 1000),
            conversion: Some(conversion),
            choice: Some(Choice::audio(
                conversion.audio_format().unwrap_or(AudioFormat::Original),
                kbps,
            )),
            ..base.clone()
        }
    };
    let mp3 = MP3_KBPS.iter().map(|kbps| {
        let suffix = if *kbps == DEFAULT_MP3_KBPS {
            "mp3".to_string()
        } else {
            format!("mp3-{kbps}")
        };
        convert(
            suffix,
            Conversion::Mp3,
            Some(*kbps),
            format!("MP3 · {kbps} kbps"),
        )
    });
    let others = [
        ("wav", Conversion::Wav, "WAV"),
        ("flac", Conversion::Flac, "FLAC"),
        ("opus", Conversion::Opus, "Opus"),
    ]
    .into_iter()
    .filter(|(_, conversion, _)| *conversion != Conversion::Opus || abilities.opus)
    .map(|(suffix, conversion, label)| convert(suffix.into(), conversion, None, label.into()));
    mp3.chain(others).collect()
}

/// Choices for one video page.
pub(crate) fn single(
    base: &MediaFormat,
    tracks: &[Track],
    abilities: Abilities,
) -> Vec<MediaFormat> {
    let any_audio = tracks.iter().any(|t| t.has_audio != Some(false));
    let mut formats = height_choices(base, tracks, abilities, any_audio);
    if formats.is_empty() {
        formats.extend(best_choices(base, tracks, abilities, any_audio));
    }
    formats.extend(original_audio(base, tracks));
    if abilities.merge && any_audio {
        formats.extend(audio_conversions(base, "ytdlp-", abilities));
    }
    formats
}

fn height_choices(
    base: &MediaFormat,
    tracks: &[Track],
    abilities: Abilities,
    any_audio: bool,
) -> Vec<MediaFormat> {
    let eligible: Vec<&Track> = tracks
        .iter()
        .filter(|t| t.has_video && t.height.is_some())
        .filter(|t| abilities.merge || t.has_audio == Some(true))
        .collect();
    let mut heights: Vec<u32> = eligible.iter().filter_map(|t| t.height).collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    let mut formats = Vec::new();
    for height in heights.into_iter().take(MAX_HEIGHTS) {
        let at_height = || eligible.iter().filter(move |t| t.height == Some(height));
        let h264 = at_height().any(|t| t.h264);
        let extension = if abilities.merge {
            "mp4".to_string()
        } else {
            at_height()
                .find_map(|t| t.extension.clone())
                .unwrap_or_else(|| "mp4".into())
        };
        let sound = if any_audio { "with sound" } else { "no sound" };
        formats.push(video(
            base,
            format!("ytdlp-{height}p"),
            format!("{height}p · {sound}"),
            &extension,
            Choice::video(Some(height), any_audio, h264),
        ));
        if !abilities.merge {
            continue;
        }
        if any_audio {
            formats.push(muted(video(
                base,
                format!("ytdlp-{height}p-mute"),
                format!("{height}p · no sound"),
                "mp4",
                Choice::video(Some(height), false, h264),
            )));
        }
        if !h264 {
            formats.push(MediaFormat {
                conversion: Some(Conversion::Mp4),
                ..video(
                    base,
                    format!("ytdlp-{height}p-h264"),
                    format!("{height}p · plays everywhere"),
                    "mp4",
                    Choice::video(Some(height), any_audio, true),
                )
            });
        }
    }
    formats
}

/// "Best quality" choices for sources that state no video heights.
fn best_choices(
    base: &MediaFormat,
    tracks: &[Track],
    abilities: Abilities,
    any_audio: bool,
) -> Vec<MediaFormat> {
    if tracks.iter().all(|t| t.audio_only) {
        return Vec::new();
    }
    let has_video = tracks.iter().any(|t| t.has_video);
    let extension = if abilities.merge && has_video {
        "mp4".to_string()
    } else {
        tracks
            .last()
            .and_then(|t| t.extension.clone())
            .unwrap_or_else(|| "mp4".into())
    };
    let best = MediaFormat {
        size_bytes: tracks.last().and_then(|t| t.size),
        ..video(
            base,
            "ytdlp-best".into(),
            "Best quality".into(),
            &extension,
            Choice::video(None, any_audio, false),
        )
    };
    let mute = (abilities.merge && any_audio).then(|| {
        muted(video(
            base,
            "ytdlp-best-mute".into(),
            "Best quality · no sound".into(),
            "mp4",
            Choice::video(None, false, false),
        ))
    });
    std::iter::once(best).chain(mute).collect()
}

/// The best audio-only track, kept in its original format.
fn original_audio(base: &MediaFormat, tracks: &[Track]) -> Option<MediaFormat> {
    let best = tracks
        .iter()
        .filter(|t| t.audio_only && t.has_audio != Some(false) && t.extension.is_some())
        .max_by(|a, b| a.audio_rate.total_cmp(&b.audio_rate))?;
    let extension = best.extension.clone()?;
    let label = if best.audio_rate > 0.0 {
        format!("Original audio · {} kbps", best.audio_rate.round() as u64)
    } else {
        "Original audio".into()
    };
    Some(MediaFormat {
        id: format!("ytdlp-audio-{extension}"),
        mime_type: mime_for(MediaKind::Audio, &extension),
        extension,
        kind: MediaKind::Audio,
        label: Some(label),
        size_bytes: best.size,
        has_audio: Some(true),
        choice: Some(Choice::audio(AudioFormat::Original, None)),
        ..base.clone()
    })
}

/// Choices for playlist entry `item` (1-based). Every choice of an entry
/// shares its label, so interfaces can name the entry from any of them.
pub(crate) fn playlist_item(item: u32, entry: &Entry, abilities: Abilities) -> Vec<MediaFormat> {
    let label = format!("{item}. {}", entry.title);
    let base = MediaFormat {
        url: entry.url.clone(),
        duration_ms: entry.duration_ms,
        item: Some(item),
        label: Some(label.clone()),
        ..MediaFormat::default()
    };
    let prefix = format!("{ITEM_PREFIX}{item}-");
    let mut formats = vec![video(
        &base,
        format!("{prefix}best"),
        label.clone(),
        "mp4",
        Choice::video(None, true, false),
    )];
    if !abilities.merge {
        return formats;
    }
    formats.push(muted(video(
        &base,
        format!("{prefix}mute"),
        label.clone(),
        "mp4",
        Choice::video(None, false, false),
    )));
    formats.push(MediaFormat {
        id: format!("{prefix}audio"),
        extension: "m4a".into(),
        mime_type: "audio/mp4".into(),
        kind: MediaKind::Audio,
        has_audio: Some(true),
        choice: Some(Choice::audio(AudioFormat::Original, None)),
        ..base.clone()
    });
    formats.extend(
        audio_conversions(&base, &prefix, abilities)
            .into_iter()
            .map(|format| MediaFormat {
                label: Some(label.clone()),
                ..format
            }),
    );
    formats
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: Abilities = Abilities {
        merge: true,
        opus: true,
    };

    fn track(height: Option<u32>, h264: bool, has_audio: Option<bool>) -> Track {
        Track {
            height,
            has_video: height.is_some(),
            audio_only: height.is_none(),
            has_audio,
            h264,
            extension: Some(if height.is_some() { "mp4" } else { "m4a" }.into()),
            audio_rate: if height.is_none() { 129.8 } else { 0.0 },
            ..Track::default()
        }
    }

    fn ids(formats: &[MediaFormat]) -> Vec<&str> {
        formats.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn every_height_has_sound_mute_and_a_plays_everywhere_choice() {
        let tracks = [
            track(Some(2160), false, Some(false)),
            track(Some(1080), true, Some(false)),
            track(None, false, Some(true)),
        ];
        let formats = single(&MediaFormat::default(), &tracks, BOTH);
        assert_eq!(
            ids(&formats),
            [
                "ytdlp-2160p",
                "ytdlp-2160p-mute",
                "ytdlp-2160p-h264",
                "ytdlp-1080p",
                "ytdlp-1080p-mute",
                "ytdlp-audio-m4a",
                "ytdlp-mp3-320",
                "ytdlp-mp3",
                "ytdlp-mp3-128",
                "ytdlp-wav",
                "ytdlp-flac",
                "ytdlp-opus",
            ]
        );
        let choice = |id: &str| {
            formats
                .iter()
                .find(|f| f.id == id)
                .unwrap()
                .choice
                .clone()
                .unwrap()
        };
        assert_eq!(
            choice("ytdlp-2160p"),
            Choice::video(Some(2160), true, false)
        );
        assert_eq!(
            choice("ytdlp-2160p-h264"),
            Choice::video(Some(2160), true, true)
        );
        assert_eq!(choice("ytdlp-1080p"), Choice::video(Some(1080), true, true));
        assert_eq!(
            choice("ytdlp-1080p-mute"),
            Choice::video(Some(1080), false, true)
        );
        assert_eq!(
            choice("ytdlp-mp3-320"),
            Choice::audio(AudioFormat::Mp3, Some(320))
        );
        assert_eq!(choice("ytdlp-audio-m4a").audio, Some(AudioFormat::Original));
    }

    #[test]
    fn without_ffmpeg_only_files_with_sound_and_the_original_audio() {
        let tracks = [
            track(Some(360), true, Some(true)),
            track(Some(1080), true, Some(false)),
            track(None, false, Some(true)),
        ];
        let formats = single(&MediaFormat::default(), &tracks, Abilities::default());
        assert_eq!(ids(&formats), ["ytdlp-360p", "ytdlp-audio-m4a"]);
    }

    #[test]
    fn opus_is_left_out_when_ffmpeg_lacks_it() {
        let tracks = [track(None, false, Some(true))];
        let abilities = Abilities {
            merge: true,
            opus: false,
        };
        let formats = single(&MediaFormat::default(), &tracks, abilities);
        assert!(!ids(&formats).contains(&"ytdlp-opus"));
        assert!(ids(&formats).contains(&"ytdlp-flac"));
    }

    #[test]
    fn sources_without_heights_get_best_and_best_without_sound() {
        let tracks = [Track {
            extension: Some("webm".into()),
            ..Track::default()
        }];
        let formats = single(&MediaFormat::default(), &tracks, BOTH);
        assert_eq!(&ids(&formats)[..2], ["ytdlp-best", "ytdlp-best-mute"]);
        let plain = single(&MediaFormat::default(), &tracks, Abilities::default());
        assert_eq!(ids(&plain), ["ytdlp-best"]);
        assert_eq!(plain[0].extension, "webm");
    }

    #[test]
    fn playlist_entries_offer_video_mute_and_every_audio_format() {
        let entry = Entry {
            url: "https://www.youtube.com/watch?v=MMxFDaIOHsE".into(),
            title: "First".into(),
            duration_ms: Some(1000),
        };
        let formats = playlist_item(3, &entry, BOTH);
        assert_eq!(
            ids(&formats),
            [
                "ytdlp-item-3-best",
                "ytdlp-item-3-mute",
                "ytdlp-item-3-audio",
                "ytdlp-item-3-mp3-320",
                "ytdlp-item-3-mp3",
                "ytdlp-item-3-mp3-128",
                "ytdlp-item-3-wav",
                "ytdlp-item-3-flac",
                "ytdlp-item-3-opus",
            ]
        );
        assert!(formats
            .iter()
            .all(|f| f.item == Some(3) && f.url == entry.url));
        assert!(formats
            .iter()
            .all(|f| f.label.as_deref() == Some("3. First")));
        assert_eq!(
            ids(&playlist_item(1, &entry, Abilities::default())),
            ["ytdlp-item-1-best"]
        );
    }
}
