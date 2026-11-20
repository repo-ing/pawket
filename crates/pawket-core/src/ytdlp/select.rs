//! From a format ID back to what to run: yt-dlp's selection arguments, and
//! the conversion Pawket's own FFmpeg step applies to the downloaded file.

use super::offer::ITEM_PREFIX;
use crate::{
    convert::{DEFAULT_MP3_KBPS, MP3_KBPS},
    Conversion,
};

/// Prefer H.264 video and AAC audio at the highest resolution allowed.
const SORT_COMPATIBLE: &str = "res,vcodec:h264,acodec:aac";
const MAX_HEIGHT: u32 = 10_000;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) args: Vec<String>,
    /// Applied by Pawket after yt-dlp finishes, with an optional bitrate.
    pub(crate) post: Option<(Conversion, Option<u32>)>,
}

impl Plan {
    fn download(args: Vec<String>) -> Self {
        Self { args, post: None }
    }

    fn then(self, conversion: Conversion, kbps: Option<u32>) -> Self {
        Self {
            post: Some((conversion, kbps)),
            ..self
        }
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

/// Best video and audio joined, or the best single file without FFmpeg.
fn joined(filter: &str, can_merge: bool, container: &str) -> Vec<String> {
    if !can_merge {
        return vec!["-f".into(), format!("b{filter}")];
    }
    vec![
        "-f".into(),
        format!("bv*{filter}+ba/b{filter}"),
        "-S".into(),
        SORT_COMPATIBLE.into(),
        "--merge-output-format".into(),
        container.into(),
    ]
}

/// The best video track; Pawket strips any audio afterwards.
fn video_only(filter: &str) -> Vec<String> {
    vec![
        "-f".into(),
        format!("bv{filter}/bv*{filter}/b{filter}"),
        "-S".into(),
        SORT_COMPATIBLE.into(),
    ]
}

fn best_audio() -> Vec<String> {
    strings(&["-f", "ba/b"])
}

/// What to run for one of the IDs `offer` produces. `None` for an unknown
/// ID or one that needs FFmpeg when it isn't configured.
pub(crate) fn plan(id: &str, can_merge: bool) -> Option<Plan> {
    let id = strip_item(id)?;
    let choice = id.strip_prefix("ytdlp-")?;
    let needs_ffmpeg = |plan: Plan| can_merge.then_some(plan);
    match choice {
        "best" => return Some(Plan::download(joined("", can_merge, "mp4"))),
        "best-mute" | "mute" => {
            return needs_ffmpeg(Plan::download(video_only("")).then(Conversion::Mute, None))
        }
        "audio" => return Some(Plan::download(best_audio())),
        "mp3" => {
            return needs_ffmpeg(
                Plan::download(best_audio()).then(Conversion::Mp3, Some(DEFAULT_MP3_KBPS)),
            )
        }
        "wav" => return needs_ffmpeg(Plan::download(best_audio()).then(Conversion::Wav, None)),
        "flac" => return needs_ffmpeg(Plan::download(best_audio()).then(Conversion::Flac, None)),
        "opus" => return needs_ffmpeg(Plan::download(best_audio()).then(Conversion::Opus, None)),
        _ => {}
    }
    if let Some(kbps) = choice.strip_prefix("mp3-") {
        let kbps: u32 = kbps.parse().ok().filter(|k| MP3_KBPS.contains(k))?;
        return needs_ffmpeg(Plan::download(best_audio()).then(Conversion::Mp3, Some(kbps)));
    }
    if let Some(ext) = choice.strip_prefix("audio-") {
        let valid =
            !ext.is_empty() && ext.len() <= 5 && ext.bytes().all(|b| b.is_ascii_alphanumeric());
        return valid.then(|| Plan::download(vec!["-f".into(), format!("ba[ext={ext}]/ba")]));
    }
    let (height, variant) = match choice.split_once("p-") {
        Some((height, variant)) => (height, Some(variant)),
        None => (choice.strip_suffix('p')?, None),
    };
    let height: u32 = height
        .parse()
        .ok()
        .filter(|h| (1..=MAX_HEIGHT).contains(h))?;
    let filter = format!("[height<={height}]");
    match variant {
        None => Some(Plan::download(joined(&filter, can_merge, "mp4"))),
        Some("mute") => {
            needs_ffmpeg(Plan::download(video_only(&filter)).then(Conversion::Mute, None))
        }
        // Matroska holds any codec pair; Pawket re-encodes it to H.264/AAC.
        Some("h264") => {
            needs_ffmpeg(Plan::download(joined(&filter, true, "mkv")).then(Conversion::Mp4, None))
        }
        Some(_) => None,
    }
}

/// `ytdlp-item-<n>-<choice>` becomes `ytdlp-<choice>`; other IDs pass
/// through. `None` when an item ID is malformed.
fn strip_item(id: &str) -> Option<String> {
    let Some(rest) = id.strip_prefix(ITEM_PREFIX) else {
        return Some(id.into());
    };
    let (number, choice) = rest.split_once('-')?;
    number.parse::<u32>().ok()?;
    Some(format!("ytdlp-{choice}"))
}

/// True when the format belongs to a playlist entry rather than the page.
pub(crate) fn is_item(id: &str) -> bool {
    id.starts_with(ITEM_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_choices_join_strip_or_re_encode() {
        let joined = plan("ytdlp-1080p", true).unwrap();
        assert_eq!(joined.args[1], "bv*[height<=1080]+ba/b[height<=1080]");
        assert!(joined
            .args
            .ends_with(&["--merge-output-format".into(), "mp4".into()]));
        assert_eq!(joined.post, None);
        let mute = plan("ytdlp-720p-mute", true).unwrap();
        assert_eq!(
            mute.args[1],
            "bv[height<=720]/bv*[height<=720]/b[height<=720]"
        );
        assert_eq!(mute.post, Some((Conversion::Mute, None)));
        let compatible = plan("ytdlp-2160p-h264", true).unwrap();
        assert!(compatible
            .args
            .ends_with(&["--merge-output-format".into(), "mkv".into()]));
        assert_eq!(compatible.post, Some((Conversion::Mp4, None)));
        assert_eq!(
            plan("ytdlp-360p", false).unwrap().args,
            ["-f", "b[height<=360]"]
        );
    }

    #[test]
    fn audio_choices_download_best_audio_then_convert() {
        assert_eq!(
            plan("ytdlp-mp3", true).unwrap().post,
            Some((Conversion::Mp3, Some(192)))
        );
        assert_eq!(
            plan("ytdlp-mp3-320", true).unwrap().post,
            Some((Conversion::Mp3, Some(320)))
        );
        assert_eq!(
            plan("ytdlp-flac", true).unwrap().post,
            Some((Conversion::Flac, None))
        );
        assert_eq!(
            plan("ytdlp-item-4-opus", true).unwrap().post,
            Some((Conversion::Opus, None))
        );
        assert_eq!(
            plan("ytdlp-item-4-audio", false).unwrap().args,
            ["-f", "ba/b"]
        );
        assert_eq!(
            plan("ytdlp-audio-m4a", false).unwrap().args,
            ["-f", "ba[ext=m4a]/ba"]
        );
        assert_eq!(plan("ytdlp-item-7-best", false).unwrap().args, ["-f", "b"]);
    }

    #[test]
    fn rejects_unknown_ids_and_ffmpeg_choices_without_ffmpeg() {
        for bad in [
            "ytdlp-mp3-256",
            "ytdlp-audio-m4a]/b",
            "ytdlp-0p",
            "ytdlp-99999p",
            "ytdlp-1080p-hdr",
            "youtube-18",
            "ytdlp-item-x-best",
            "ytdlp-item-3",
        ] {
            assert_eq!(plan(bad, true), None, "accepted {bad}");
        }
        for needs_ffmpeg in [
            "ytdlp-mp3",
            "ytdlp-wav",
            "ytdlp-1080p-mute",
            "ytdlp-1080p-h264",
            "ytdlp-item-2-mute",
        ] {
            assert_eq!(plan(needs_ffmpeg, false), None, "{needs_ffmpeg}");
        }
        assert!(is_item("ytdlp-item-2-best") && !is_item("ytdlp-best"));
    }
}
