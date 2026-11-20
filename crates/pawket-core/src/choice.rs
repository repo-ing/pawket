//! A structured description of each format, so interfaces can offer simple
//! pickers (video or audio, quality, sound, format) instead of long lists.

use crate::{Conversion, MediaFormat, MediaKind};
use serde::{Deserialize, Serialize};

/// The audio file a format produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    /// The source's own audio track, unconverted.
    Original,
    Mp3,
    Wav,
    Flac,
    Opus,
}

/// What a format is, in the terms a picker shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub mode: MediaKind,
    /// Video height in pixels, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Video only: the file includes sound.
    #[serde(default)]
    pub sound: bool,
    /// Video only: an H.264/AAC MP4 that plays in every common player.
    #[serde(default)]
    pub compatible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioFormat>,
    /// Target bitrate for encoded audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kbps: Option<u32>,
}

impl Choice {
    pub(crate) fn video(height: Option<u32>, sound: bool, compatible: bool) -> Self {
        Self {
            mode: MediaKind::Video,
            height,
            sound,
            compatible,
            ..Self::default()
        }
    }

    pub(crate) fn audio(audio: AudioFormat, kbps: Option<u32>) -> Self {
        Self {
            mode: MediaKind::Audio,
            sound: true,
            audio: Some(audio),
            kbps,
            ..Self::default()
        }
    }
}

impl Conversion {
    pub(crate) fn audio_format(self) -> Option<AudioFormat> {
        match self {
            Self::Mp3 => Some(AudioFormat::Mp3),
            Self::Wav => Some(AudioFormat::Wav),
            Self::Flac => Some(AudioFormat::Flac),
            Self::Opus => Some(AudioFormat::Opus),
            Self::Mp4 | Self::Mute => None,
        }
    }
}

/// Describe a format from the fields its provider already filled in.
pub(crate) fn infer(format: &MediaFormat, provider: &str) -> Choice {
    match format.kind {
        MediaKind::Image => Choice {
            mode: MediaKind::Image,
            ..Choice::default()
        },
        MediaKind::Audio => {
            let audio = format
                .conversion
                .and_then(Conversion::audio_format)
                .unwrap_or(AudioFormat::Original);
            let kbps = (audio == AudioFormat::Mp3)
                .then(|| format.bitrate.map(|bits| (bits / 1000) as u32))
                .flatten();
            Choice::audio(audio, kbps)
        }
        MediaKind::Video => {
            let sound =
                format.has_audio != Some(false) && format.conversion != Some(Conversion::Mute);
            // X serves H.264/AAC MP4s; an explicit MP4 conversion encodes to it.
            let compatible = format.conversion == Some(Conversion::Mp4)
                || (provider == "x" && format.extension == "mp4");
            Choice::video(format.height, sound, compatible)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_videos_audio_and_photos() {
        let video = MediaFormat {
            extension: "mp4".into(),
            height: Some(720),
            ..MediaFormat::default()
        };
        assert_eq!(infer(&video, "x"), Choice::video(Some(720), true, true));
        assert_eq!(
            infer(&video, "html_media"),
            Choice::video(Some(720), true, false)
        );
        let muted = MediaFormat {
            conversion: Some(Conversion::Mute),
            ..video.clone()
        };
        assert!(!infer(&muted, "x").sound);
        let mp3 = MediaFormat {
            kind: MediaKind::Audio,
            conversion: Some(Conversion::Mp3),
            bitrate: Some(320_000),
            ..MediaFormat::default()
        };
        assert_eq!(infer(&mp3, "x"), Choice::audio(AudioFormat::Mp3, Some(320)));
        let original = MediaFormat {
            kind: MediaKind::Audio,
            ..MediaFormat::default()
        };
        assert_eq!(infer(&original, "x").audio, Some(AudioFormat::Original));
        let photo = MediaFormat {
            kind: MediaKind::Image,
            ..MediaFormat::default()
        };
        assert_eq!(infer(&photo, "x").mode, MediaKind::Image);
    }

    #[test]
    fn serializes_compactly_for_clients() {
        let json = serde_json::to_value(Choice::audio(AudioFormat::Flac, None)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"mode":"audio","sound":true,"compatible":false,"audio":"flac"})
        );
    }
}
