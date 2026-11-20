use crate::{
    storage, Conversion, MediaFormat, MediaInfo, MediaKind, PawketError, Result, MAX_MEDIA_BYTES,
};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tempfile::TempPath;
use tokio::{fs::File, process::Command, time::timeout};

/// Re-encoding long or high-resolution video to H.264 can take a while.
const ENCODE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// MP3 bitrates offered, best first. The middle one is the default.
pub(crate) const MP3_KBPS: [u32; 3] = [320, 192, 128];
pub(crate) const DEFAULT_MP3_KBPS: u32 = 192;
const OPUS_KBPS: u32 = 160;

/// Optional encoders found in the configured FFmpeg.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Encoders {
    pub(crate) opus: bool,
}

impl Conversion {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Mp4 | Self::Mute => "mp4",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Opus => "opus",
        }
    }

    fn mime_type(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Mp4 | Self::Mute => "video/mp4",
            Self::Wav => "audio/wav",
            Self::Flac => "audio/flac",
            Self::Opus => "audio/ogg",
        }
    }

    fn is_audio(self) -> bool {
        self.audio_format().is_some()
    }

    fn label(self, kbps: Option<u32>) -> String {
        match (self, kbps) {
            (Self::Mp3, Some(kbps)) => format!("MP3 · {kbps} kbps"),
            (Self::Mp3, None) => "MP3".into(),
            (Self::Mp4, _) => "Convert to MP4".into(),
            (Self::Mute, _) => "No sound".into(),
            (Self::Wav, _) => "WAV".into(),
            (Self::Flac, _) => "FLAC".into(),
            (Self::Opus, _) => format!("Opus · {OPUS_KBPS} kbps"),
        }
    }

    /// The conversion of `source` this choice produces. MP3 at the default
    /// bitrate keeps the short `-as-mp3` ID used by earlier releases.
    fn format(self, source: &MediaFormat, kbps: Option<u32>) -> MediaFormat {
        let suffix = match (self, kbps) {
            (Self::Mp3, Some(kbps)) if kbps != DEFAULT_MP3_KBPS => format!("as-mp3-{kbps}"),
            (Self::Mute, _) => "muted".into(),
            _ => format!("as-{}", self.extension()),
        };
        let label = self.label(kbps);
        let mut format = MediaFormat {
            id: format!("{}-{suffix}", source.id),
            extension: self.extension().into(),
            mime_type: self.mime_type().into(),
            size_bytes: None,
            bitrate: kbps.map(|kbps| u64::from(kbps) * 1000),
            conversion: Some(self),
            label: Some(match (&source.label, self.is_audio()) {
                (Some(source_label), false) => format!("{label} · {source_label}"),
                _ => label,
            }),
            ..source.clone()
        };
        if self.is_audio() {
            format.kind = MediaKind::Audio;
            format.width = None;
            format.height = None;
            format.has_audio = Some(true);
        }
        if self == Self::Mute {
            format.has_audio = Some(false);
        }
        format
    }
}

/// Every audio conversion offered from one source, best first.
fn audio_conversions(source: &MediaFormat, encoders: Encoders) -> Vec<MediaFormat> {
    let mp3 = MP3_KBPS
        .iter()
        .map(|kbps| Conversion::Mp3.format(source, Some(*kbps)));
    let lossless = [Conversion::Wav, Conversion::Flac]
        .into_iter()
        .map(|conversion| conversion.format(source, None));
    let opus = encoders
        .opus
        .then(|| Conversion::Opus.format(source, Some(OPUS_KBPS)));
    mp3.chain(lossless).chain(opus).collect()
}

/// The best original with sound for each media item: an audio file when
/// there is one, else the first (best) video with sound.
fn audio_sources(formats: &[MediaFormat]) -> Vec<&MediaFormat> {
    let originals: Vec<&MediaFormat> = formats.iter().filter(|f| f.conversion.is_none()).collect();
    let mut items: Vec<Option<u32>> = originals.iter().map(|f| f.item).collect();
    items.dedup();
    items.sort_unstable();
    items.dedup();
    items
        .into_iter()
        .filter_map(|item| {
            let in_item = || originals.iter().filter(move |f| f.item == item);
            in_item()
                .find(|f| f.kind == MediaKind::Audio)
                .or_else(|| {
                    in_item().find(|f| f.kind == MediaKind::Video && f.has_audio != Some(false))
                })
                .copied()
        })
        .collect()
}

pub(crate) fn add_formats(info: &mut MediaInfo, encoders: Encoders) {
    let mut conversions = Vec::new();
    for source in info
        .formats
        .iter()
        .filter(|format| format.conversion.is_none() && format.kind == MediaKind::Video)
    {
        if source.extension != "mp4" {
            conversions.push(Conversion::Mp4.format(source, None));
        }
        if source.has_audio != Some(false) {
            conversions.push(Conversion::Mute.format(source, None));
        }
    }
    for source in audio_sources(&info.formats) {
        conversions.extend(audio_conversions(source, encoders));
    }
    info.formats.extend(conversions);
}

async fn list_feature(executable: &Path, option: &str) -> Result<String> {
    let result = timeout(
        Duration::from_secs(5),
        Command::new(executable)
            .args(["-hide_banner", option])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| PawketError::ConverterUnavailable)?
    .map_err(|_| PawketError::ConverterUnavailable)?;
    if !result.status.success() {
        return Err(PawketError::ConverterUnavailable);
    }
    String::from_utf8(result.stdout).map_err(|_| PawketError::ConverterUnavailable)
}

fn has_encoder(encoders: &str, name: &str) -> bool {
    encoders
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some(name))
}

pub(crate) async fn check_executable(executable: PathBuf) -> Result<(PathBuf, Encoders)> {
    let executable = tokio::fs::canonicalize(executable)
        .await
        .map_err(|_| PawketError::ConverterUnavailable)?;
    let protocols = list_feature(&executable, "-protocols").await?;
    let encoders = list_feature(&executable, "-encoders").await?;
    // FLAC and 16-bit PCM are built into every FFmpeg; these are not.
    let required = ["libmp3lame", "libx264", "aac", "flac", "pcm_s16le"];
    if protocols.lines().filter(|line| line.trim() == "fd").count() < 2
        || !required.iter().all(|name| has_encoder(&encoders, name))
    {
        return Err(PawketError::ConverterUnavailable);
    }
    let found = Encoders {
        opus: has_encoder(&encoders, "libopus"),
    };
    Ok((executable, found))
}

/// Encoder arguments for one conversion of a single input.
fn encode_args(conversion: Conversion, kbps: Option<u32>) -> Vec<String> {
    let audio = |codec: &str| vec!["-map".into(), "0:a:0".into(), "-c:a".into(), codec.into()];
    let mut args = match conversion {
        Conversion::Mp3 => audio("libmp3lame"),
        Conversion::Wav => audio("pcm_s16le"),
        Conversion::Flac => audio("flac"),
        Conversion::Opus => audio("libopus"),
        Conversion::Mute => ["-map", "0:V:0", "-c:v", "copy", "-an"]
            .map(String::from)
            .to_vec(),
        Conversion::Mp4 => [
            "-map",
            "0:V:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-pix_fmt",
            "yuv420p",
            "-threads",
            "2",
            "-filter_threads",
            "2",
            "-vf",
            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
        ]
        .map(String::from)
        .to_vec(),
    };
    match (conversion, kbps) {
        (Conversion::Mp3, Some(kbps)) | (Conversion::Opus, Some(kbps)) => {
            args.extend(["-b:a".into(), format!("{kbps}k")]);
        }
        (Conversion::Mp3, None) => args.extend(["-q:a".into(), "2".into()]),
        (Conversion::Opus, None) => args.extend(["-b:a".into(), format!("{OPUS_KBPS}k")]),
        _ => {}
    }
    args
}

pub(crate) async fn run(
    executable: &Path,
    input: &Path,
    separate_audio: Option<&Path>,
    directory: &Path,
    conversion: Conversion,
    kbps: Option<u32>,
) -> Result<(TempPath, u64)> {
    if separate_audio.is_some() && conversion != Conversion::Mp4 {
        return Err(PawketError::FormatUnavailable);
    }
    let input = File::open(input).await?.into_std().await;
    let (output, path) = storage::create_temporary(directory).await?;
    let output = output.into_std().await;
    let mut command = Command::new(executable);
    // FFmpeg only receives opened descriptors. No path or URL from a format,
    // page, or media container is sent to the subprocess.
    command.args([
        "-hide_banner",
        "-loglevel",
        "quiet",
        "-nostdin",
        "-xerror",
        "-abort_on",
        "empty_output+empty_output_stream",
        "-protocol_whitelist",
        "fd",
        "-format_whitelist",
        "mov,matroska,webm,ogg,mp3,aac,wav,flac",
        "-threads",
        "2",
        "-max_pixels",
        "67108864",
        "-fd",
        "0",
        "-i",
        "fd:",
    ]);
    let stderr = if let Some(audio) = separate_audio {
        // Descriptor 2 is a read-only media input for this quiet invocation.
        // This keeps both inputs seekable without enabling the file protocol.
        command.args([
            "-protocol_whitelist",
            "fd",
            "-format_whitelist",
            "mov",
            "-fd",
            "2",
            "-i",
            "fd:",
        ]);
        Stdio::from(File::open(audio).await?.into_std().await)
    } else {
        Stdio::null()
    };
    command.args(["-map_metadata", "-1", "-map_chapters", "-1"]);
    if separate_audio.is_some() {
        command.args(["-map", "0:V:0", "-map", "1:a:0", "-c", "copy"]);
    } else {
        command.args(encode_args(conversion, kbps));
    }
    command
        .arg("-fs")
        .arg(MAX_MEDIA_BYTES.to_string())
        .args(["-f", conversion.extension(), "-fd", "1", "fd:"])
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output))
        .stderr(stderr)
        .kill_on_drop(true);

    let status = timeout(ENCODE_TIMEOUT, command.status())
        .await
        .map_err(|_| PawketError::ConversionFailed)?
        .map_err(|_| PawketError::ConverterUnavailable)?;
    let output = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .await?;
    let bytes = output.metadata().await?.len();
    if bytes >= MAX_MEDIA_BYTES {
        return Err(PawketError::TooLarge {
            limit: MAX_MEDIA_BYTES,
        });
    }
    if !status.success() || bytes == 0 {
        return Err(PawketError::ConversionFailed);
    }
    output.sync_all().await?;
    Ok((path, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(media: &MediaInfo) -> Vec<&str> {
        media.formats.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn offers_mute_mp4_and_every_audio_format_from_the_best_source() {
        let mut media = MediaInfo {
            formats: vec![
                MediaFormat {
                    id: "clip".into(),
                    extension: "webm".into(),
                    size_bytes: Some(100),
                    width: Some(640),
                    ..MediaFormat::default()
                },
                MediaFormat {
                    id: "small".into(),
                    extension: "mp4".into(),
                    ..MediaFormat::default()
                },
                MediaFormat {
                    id: "silent".into(),
                    extension: "mp4".into(),
                    has_audio: Some(false),
                    ..MediaFormat::default()
                },
            ],
            ..MediaInfo::default()
        };
        add_formats(&mut media, Encoders { opus: true });
        assert_eq!(
            ids(&media),
            [
                "clip",
                "small",
                "silent",
                "clip-as-mp4",
                "clip-muted",
                "small-muted",
                "clip-as-mp3-320",
                "clip-as-mp3",
                "clip-as-mp3-128",
                "clip-as-wav",
                "clip-as-flac",
                "clip-as-opus",
            ]
        );
        let mp3 = &media.formats[6];
        assert_eq!(
            (mp3.kind, mp3.width, mp3.bitrate),
            (MediaKind::Audio, None, Some(320_000))
        );
        assert_eq!(mp3.label.as_deref(), Some("MP3 · 320 kbps"));
        assert_eq!(media.formats[4].has_audio, Some(false));
        assert_eq!(media.formats[0].size_bytes, Some(100));
    }

    #[test]
    fn audio_comes_from_each_items_audio_file_or_first_video_with_sound() {
        let format = |id: &str, item, kind, has_audio| MediaFormat {
            id: id.into(),
            item,
            kind,
            has_audio,
            extension: "mp4".into(),
            ..MediaFormat::default()
        };
        let formats = vec![
            format("v1-hd", Some(1), MediaKind::Video, None),
            format("v1-sd", Some(1), MediaKind::Video, None),
            format("gif", Some(2), MediaKind::Video, Some(false)),
            format("photo", Some(3), MediaKind::Image, None),
            format("v4", Some(4), MediaKind::Video, None),
            format("a4", Some(4), MediaKind::Audio, None),
        ];
        let sources: Vec<_> = audio_sources(&formats)
            .iter()
            .map(|f| f.id.as_str())
            .collect();
        assert_eq!(sources, ["v1-hd", "a4"]);
    }

    #[test]
    fn opus_is_offered_only_when_ffmpeg_has_it() {
        let source = MediaFormat {
            id: "a".into(),
            kind: MediaKind::Audio,
            ..MediaFormat::default()
        };
        assert!(!audio_conversions(&source, Encoders::default())
            .iter()
            .any(|f| f.conversion == Some(Conversion::Opus)));
    }

    #[test]
    fn encoder_arguments_match_each_choice() {
        assert!(encode_args(Conversion::Mp3, Some(320)).ends_with(&["-b:a".into(), "320k".into()]));
        assert!(encode_args(Conversion::Mp3, None).ends_with(&["-q:a".into(), "2".into()]));
        assert!(encode_args(Conversion::Wav, None).contains(&"pcm_s16le".to_string()));
        assert!(encode_args(Conversion::Mute, None).contains(&"-an".to_string()));
        assert!(encode_args(Conversion::Opus, None).contains(&"160k".to_string()));
    }

    #[tokio::test]
    #[ignore = "requires a local FFmpeg build with fd protocol and encoding support"]
    async fn converts_real_tracks_and_refuses_playlist_inputs() {
        let (executable, encoders) = check_executable(
            std::env::var_os("PAWKET_TEST_FFMPEG")
                .expect("set PAWKET_TEST_FFMPEG to the FFmpeg executable path")
                .into(),
        )
        .await
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join("fixture.webm");
        let generated = Command::new(&executable)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440",
                "-t",
                "1",
                "-c:v",
                "libvpx-vp9",
                "-c:a",
                "libopus",
            ])
            .arg(&fixture)
            .status()
            .await
            .unwrap();
        assert!(generated.success());

        let mut cases = vec![
            (Conversion::Mp3, Some(320), &["0:a:0"][..], "mp3"),
            (Conversion::Mp3, None, &["0:a:0"][..], "mp3"),
            (Conversion::Wav, None, &["0:a:0"][..], "pcm_s16le"),
            (Conversion::Flac, None, &["0:a:0"][..], "flac"),
            (Conversion::Mp4, None, &["0:v:0", "0:a:0"][..], "h264"),
            (Conversion::Mute, None, &["0:v:0"][..], "vp9"),
        ];
        if encoders.opus {
            cases.push((Conversion::Opus, None, &["0:a:0"][..], "opus"));
        }
        for (format, kbps, tracks, codec) in cases {
            let (output, bytes) = run(&executable, &fixture, None, directory.path(), format, kbps)
                .await
                .unwrap();
            assert!(bytes > 100, "{format:?}");
            let mut decode = Command::new(&executable);
            decode.args(["-v", "error", "-xerror", "-i"]).arg(&*output);
            for track in tracks {
                decode.args(["-map", track]);
            }
            assert!(decode
                .args(["-f", "null", "-"])
                .status()
                .await
                .unwrap()
                .success());
            let probe = Command::new(executable.with_file_name("ffprobe"))
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=codec_name,codec_type",
                ])
                .args(["-of", "csv=p=0"])
                .arg(&*output)
                .output()
                .await
                .unwrap();
            let streams = String::from_utf8_lossy(&probe.stdout);
            assert!(streams.contains(codec), "{format:?}: {streams}");
            if format == Conversion::Mute {
                assert!(!streams.contains("audio"), "muted output kept audio");
            }
        }

        let video = directory.path().join("video.mp4");
        let audio = directory.path().join("audio.m4a");
        for (track, encoder, path) in [("0:v:0", "libx264", &video), ("0:a:0", "aac", &audio)] {
            let status = Command::new(&executable)
                .args(["-v", "error", "-i"])
                .arg(&fixture)
                .args(["-map", track, "-c", encoder])
                .arg(path)
                .status()
                .await
                .unwrap();
            assert!(status.success());
        }
        let (joined, _) = run(
            &executable,
            &video,
            Some(&audio),
            directory.path(),
            Conversion::Mp4,
            None,
        )
        .await
        .unwrap();
        let status = Command::new(&executable)
            .args(["-v", "error", "-xerror", "-i"])
            .arg(&*joined)
            .args(["-map", "0:v:0", "-map", "0:a:0", "-f", "null", "-"])
            .status()
            .await
            .unwrap();
        assert!(status.success());
        drop(joined);

        let playlist = directory.path().join("playlist");
        tokio::fs::write(
            &playlist,
            format!("ffconcat version 1.0\nfile '{}'\n", fixture.display()),
        )
        .await
        .unwrap();
        assert!(matches!(
            run(
                &executable,
                &playlist,
                None,
                directory.path(),
                Conversion::Mp4,
                None
            )
            .await,
            Err(PawketError::ConversionFailed)
        ));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 4);
    }
}
