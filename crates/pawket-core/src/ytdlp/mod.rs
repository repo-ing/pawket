//! Optional yt-dlp backend: site extractors, streamed formats and playlists.
//!
//! Pawket runs a local yt-dlp executable the operator configured. Every
//! connection yt-dlp makes goes through a loopback proxy that applies the
//! same public-HTTPS rules as native downloads (see `proxy`). Files are committed through Pawket’s no-overwrite storage.

mod cookies;
mod offer;
mod parse;
mod process;
mod proxy;
mod routes;
mod select;
mod worker;

pub use cookies::CookieSource;
pub(crate) use offer::{single, Abilities, Track};
pub(crate) use parse::{MAX_ENTRIES, PROVIDER};

use crate::{
    convert, network, storage, DownloadResult, MediaFormat, MediaInfo, PawketError, Progress,
    Result,
};
use process::{read_capped, Run};
use proxy::GuardProxy;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::timeout,
};
use url::Url;

const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const INSPECT_TIMEOUT: Duration = Duration::from_secs(90);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long a process may linger after closing its output.
const EXIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const OUTPUT_TEMPLATE: &str = "%(title).120B.%(ext)s";
const PROGRESS_TEMPLATE: &str = "download:PAWKET %(progress.downloaded_bytes)s \
     %(progress.total_bytes)s %(progress.total_bytes_estimate)s";
/// Partial-file suffixes yt-dlp leaves while it works.
const PARTIAL_SUFFIXES: [&str; 4] = [".part", ".ytdl", ".temp", ".frag"];
/// Variables that change which Python code yt-dlp loads.
const PYTHON_VARIABLES: [&str; 5] = [
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONSTARTUP",
    "PYTHONUSERBASE",
    "PYTHONINSPECT",
];
/// Options that keep local configuration out of Pawket's runs: config
/// files can add --exec, cookies or output paths, and plugins run code.
const ISOLATION: [&str; 2] = ["--ignore-config", "--no-plugin-dirs"];

/// A checked yt-dlp executable.
#[derive(Clone, Debug)]
pub(crate) struct Backend {
    worker: std::sync::Arc<worker::AudioWorker>,
    executable: PathBuf,
    version: String,
    cookies: Option<CookieSource>,
}

impl Backend {
    pub(crate) async fn warm_audio(&self) {
        if self.cookies.is_none() {
            self.worker.warm(self).await;
        }
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    /// Sign runs in with these cookies.
    pub(crate) fn with_cookies(self, cookies: CookieSource) -> Self {
        Self {
            cookies: Some(cookies),
            ..self
        }
    }

    pub(crate) fn cookies(&self) -> Option<&CookieSource> {
        self.cookies.as_ref()
    }

    fn error_from(&self, stderr: &[u8]) -> PawketError {
        parse::error_from(&String::from_utf8_lossy(stderr), self.cookies.is_some())
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        for variable in PYTHON_VARIABLES {
            command.env_remove(variable);
        }
        command.args(ISOLATION).stdin(Stdio::null());
        command
    }

    /// A run whose every connection goes through `proxy`. Native
    /// downloaders only, so no external tool fetches around the proxy.
    fn command(&self, proxy: &GuardProxy) -> Command {
        let mut command = self.base_command();
        command
            .args(["--no-warnings", "--color", "never", "--no-playlist"])
            .args([
                "--socket-timeout",
                "30",
                "--downloader",
                "native",
                "--proxy",
            ])
            .arg(proxy.url())
            .args(self.cookies.iter().flat_map(CookieSource::args))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
}

pub(crate) async fn check_executable(executable: PathBuf) -> Result<Backend> {
    let executable = tokio::fs::canonicalize(executable)
        .await
        .map_err(|_| PawketError::BackendUnavailable)?;
    let backend = Backend {
        worker: Default::default(),
        executable,
        version: String::new(),
        cookies: None,
    };
    let mut command = backend.base_command();
    command.arg("--version").kill_on_drop(true);
    let output = timeout(VERSION_TIMEOUT, command.output())
        .await
        .map_err(|_| PawketError::BackendUnavailable)?
        .map_err(|_| PawketError::BackendUnavailable)?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let plausible = !version.is_empty()
        && version.len() <= 32
        && version.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    // Builds too old for --no-plugin-dirs fail here and must be updated.
    if !output.status.success() || !plausible {
        return Err(PawketError::BackendUnavailable);
    }
    Ok(Backend { version, ..backend })
}

/// Inspect a link. The flag is true when only yt-dlp's generic page scan
/// recognised it.
pub(crate) async fn inspect(
    backend: &Backend,
    source: &Url,
    abilities: Abilities,
) -> Result<(MediaInfo, bool)> {
    let (url, route_args) = routed_link(source.as_str()).await?;
    let proxy = GuardProxy::start().await?;
    let mut command = backend.command(&proxy);
    command
        .args(["-J", "--flat-playlist", "--playlist-end"])
        .arg(MAX_ENTRIES.to_string())
        .args(&route_args)
        .arg("--")
        .arg(url.as_str());
    let mut run = Run::spawn(command)?;
    let (stdout, stderr) = (run.stdout()?, run.stderr()?);
    let read = async {
        tokio::join!(
            read_capped(stdout, MAX_JSON_BYTES),
            read_capped(stderr, MAX_STDERR_BYTES)
        )
    };
    let ((json, json_overflowed), (errors, _)) = timeout(INSPECT_TIMEOUT, read)
        .await
        .map_err(|_| PawketError::Network("yt-dlp took too long to read that link".into()))?;
    let status = run.finish(EXIT_TIMEOUT).await?;
    if !status.success() {
        return Err(backend.error_from(&errors));
    }
    if json_overflowed {
        return Err(PawketError::TooLarge {
            limit: MAX_JSON_BYTES as u64,
        });
    }
    let info: parse::Info = serde_json::from_slice(&json).map_err(|_| {
        PawketError::SourceUnavailable("yt-dlp returned unreadable details.".into())
    })?;
    let generic = info.is_generic();
    parse::to_media_info(&url, info, abilities).map(|media| (media, generic))
}

/// A link that passes Pawket's URL rules and whose host is public now. The
/// proxy re-checks every connection, including this one.
async fn checked_link(link: &str) -> Result<Url> {
    let url = network::validate_url(link)?;
    network::public_addresses(&url).await?;
    Ok(url)
}

/// A checked link moved to the site's public route where it has one (for
/// example Vimeo's player), with the arguments that route needs.
async fn routed_link(link: &str) -> Result<(Url, Vec<String>)> {
    let (url, args) = routes::public_route(&checked_link(link).await?);
    Ok((checked_link(url.as_str()).await?, args))
}

/// Bytes across the files of one run; yt-dlp restarts its counter for each
/// track it fetches before joining them.
#[derive(Default)]
struct Tally {
    finished: u64,
    current: u64,
}

impl Tally {
    fn update(&mut self, downloaded: u64) -> u64 {
        if downloaded < self.current {
            self.finished = self.finished.saturating_add(self.current);
        }
        self.current = downloaded;
        self.finished.saturating_add(downloaded)
    }
}

pub(crate) async fn download(
    backend: &Backend,
    ffmpeg: Option<&Path>,
    info: &MediaInfo,
    format: &MediaFormat,
    directory: &Path,
    progress: &Progress,
) -> Result<DownloadResult> {
    if matches!(
        format.id.as_str(),
        "ytdlp-quick-mp3" | "ytdlp-quick-original"
    ) && backend.cookies().is_none()
    {
        match download_with_options(backend, ffmpeg, info, format, directory, progress, true).await
        {
            Err(PawketError::FormatUnavailable) => {}
            other => return other,
        }
    }
    download_with_options(backend, ffmpeg, info, format, directory, progress, false).await
}

async fn download_with_options(
    backend: &Backend,
    ffmpeg: Option<&Path>,
    info: &MediaInfo,
    format: &MediaFormat,
    directory: &Path,
    progress: &Progress,
    direct_audio: bool,
) -> Result<DownloadResult> {
    let plan = select::plan(&format.id, ffmpeg.is_some()).ok_or(if ffmpeg.is_none() {
        PawketError::ConverterUnavailable
    } else {
        PawketError::FormatUnavailable
    })?;
    let (url, route_args) = routed_link(&format.url).await?;
    tokio::fs::create_dir_all(directory).await?;
    let work = tempfile::Builder::new()
        .prefix(".pawket-ytdlp-")
        .tempdir_in(directory)?;
    // Isolate partial files from a worker that exits or declines a source.
    let worker_dir = tempfile::tempdir_in(work.path())?;
    let used_worker = if direct_audio {
        match crate::youtube::video_id(&url) {
            Some(id) => {
                backend
                    .worker
                    .download(backend, &id, &worker_dir.path().canonicalize()?, progress)
                    .await?
                    && finished_file(worker_dir.path()).await.is_ok()
            }
            None => false,
        }
    } else {
        false
    };
    if !used_worker {
        let proxy = GuardProxy::start().await?;

        let mut command = backend.command(&proxy);
        command
            .args([
                "--newline",
                "--progress",
                "--progress-template",
                PROGRESS_TEMPLATE,
            ])
            .args([
                "--no-mtime",
                "--concurrent-fragments",
                "4",
                "--playlist-items",
                "1",
            ])
            // -P takes a plain path; only the -o template is expanded.
            .arg("-P")
            .arg(work.path())
            .args(["-o", OUTPUT_TEMPLATE])
            .args(&route_args);
        if let Some(ffmpeg) = ffmpeg {
            command.arg("--ffmpeg-location").arg(ffmpeg);
        }
        if matches!(
            format.id.as_str(),
            "ytdlp-quick-mp3" | "ytdlp-quick-original"
        ) {
            command.args([
                "--match-filter",
                "live_status!=?is_live & live_status!=?is_upcoming",
            ]);
        }
        if direct_audio {
            // Prefer a direct audio track for the shortcut. A manifest-only video
            // is retried with yt-dlp's full default set of clients and protocols.
            command.args([
                "--extractor-args",
                "youtube:player_client=tv_samsung;skip=hls,dash",
            ]);
        }
        command.args(&plan.args).arg("--").arg(url.as_str());

        let mut run = Run::spawn(command)?;
        let (stdout, stderr) = (run.stdout()?, run.stderr()?);
        let errors = tokio::spawn(read_capped(stderr, MAX_STDERR_BYTES));
        timeout(DOWNLOAD_TIMEOUT, watch_progress(stdout, progress))
            .await
            .map_err(|_| PawketError::Network("The download took too long".into()))??;
        // Any error above drops `run`, which stops yt-dlp and its FFmpeg.
        let status = run.finish(EXIT_TIMEOUT).await?;
        let (errors, _) = errors.await.unwrap_or_default();
        if !status.success() {
            if direct_audio {
                let message = String::from_utf8_lossy(&errors);
                if message.contains("Requested format is not available")
                    || message.contains("No video formats found")
                    || message.contains("HTTP Error 403")
                {
                    return Err(PawketError::FormatUnavailable);
                }
            }
            return Err(backend.error_from(&errors));
        }
    }
    let (file, bytes) = finished_file(if used_worker {
        worker_dir.path()
    } else {
        work.path()
    })
    .await?;
    let title = if matches!(
        format.id.as_str(),
        "ytdlp-quick-mp3" | "ytdlp-quick-original"
    ) || select::is_item(&format.id)
    {
        file.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&info.title)
            .to_string()
    } else {
        info.title.clone()
    };
    let (file, extension, bytes, _converted) = match (plan.post, ffmpeg) {
        (None, _) => {
            let extension = file
                .extension()
                .and_then(|ext| ext.to_str())
                .ok_or(PawketError::NotMedia)?
                .to_string();
            (file, extension, bytes, None)
        }
        (Some((conversion, kbps)), Some(ffmpeg)) => {
            progress.set_total(None);
            let (converted, bytes) = convert::run_with_effort(
                ffmpeg,
                &file,
                None,
                work.path(),
                conversion,
                kbps,
                (format.id == "ytdlp-quick-mp3").then_some(7),
            )
            .await?;
            let path = converted.to_path_buf();
            (
                path,
                conversion.extension().to_string(),
                bytes,
                Some(converted),
            )
        }
        (Some(_), None) => return Err(PawketError::ConverterUnavailable),
    };
    progress.set_written(bytes);
    progress.set_total(Some(bytes));
    let path = storage::commit_temporary(&file, directory, &title, &extension).await?;
    // `work` is removed on drop; the committed hard link keeps the file.
    drop(work);
    Ok(DownloadResult {
        filename: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: path.to_string_lossy().into_owned(),
        bytes,
        source_url: info.source_url.clone(),
    })
}

/// Follow yt-dlp's progress lines.
async fn watch_progress(stdout: tokio::process::ChildStdout, progress: &Progress) -> Result<()> {
    let mut lines = BufReader::new(stdout).lines();
    let mut tally = Tally::default();
    while let Some(line) = lines.next_line().await? {
        let Some((downloaded, total)) = parse::progress_line(&line) else {
            continue;
        };
        let written = tally.update(downloaded);
        progress.set_written(written);
        progress.set_total(if tally.finished == 0 { total } else { None });
    }
    Ok(())
}

/// The one finished file yt-dlp left in its work directory.
async fn finished_file(work: &Path) -> Result<(PathBuf, u64)> {
    let mut entries = tokio::fs::read_dir(work).await?;
    let mut best: Option<(PathBuf, u64)> = None;
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let partial = PARTIAL_SUFFIXES.iter().any(|suffix| name.ends_with(suffix));
        if !metadata.is_file() || partial {
            continue;
        }
        if best.as_ref().is_none_or(|(_, size)| metadata.len() > *size) {
            best = Some((entry.path(), metadata.len()));
        }
    }
    let (path, bytes) = best.ok_or_else(|| {
        PawketError::SourceUnavailable("yt-dlp finished without saving a file.".into())
    })?;
    if bytes == 0 {
        return Err(PawketError::NotMedia);
    }
    Ok((path, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_adds_up_separate_tracks() {
        let mut tally = Tally::default();
        assert_eq!(tally.update(100), 100);
        assert_eq!(tally.update(300), 300);
        // The audio track starts from zero after the video finished.
        assert_eq!(tally.update(50), 350);
        assert_eq!(tally.update(80), 380);
    }

    #[tokio::test]
    async fn picks_the_finished_file_and_ignores_partials() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("clip.mp4"), b"video")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("clip.f137.mp4.part"), b"partial-bigger")
            .await
            .unwrap();
        let (path, bytes) = finished_file(dir.path()).await.unwrap();
        assert_eq!(
            (path.file_name().unwrap().to_str(), bytes),
            (Some("clip.mp4"), 5)
        );
    }

    #[tokio::test]
    async fn an_empty_work_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(finished_file(dir.path()).await.is_err());
    }

    fn live_backend() -> Option<PathBuf> {
        std::env::var_os("PAWKET_TEST_YTDLP").map(PathBuf::from)
    }

    #[tokio::test]
    #[ignore = "requires PAWKET_TEST_YTDLP, PAWKET_TEST_FFMPEG and direct HTTPS access"]
    async fn downloads_into_a_percent_directory_through_the_proxy() {
        let backend = check_executable(live_backend().unwrap()).await.unwrap();
        let ffmpeg = std::env::var_os("PAWKET_TEST_FFMPEG").map(PathBuf::from);
        let source = Url::parse("https://www.youtube.com/watch?v=jNQXAC9IVRw").unwrap();
        let abilities = Abilities {
            merge: ffmpeg.is_some(),
            opus: false,
        };
        let (info, _) = inspect(&backend, &source, abilities).await.unwrap();
        let format = info.formats.last().unwrap();
        let parent = tempfile::tempdir().unwrap();
        // A literal %(...)s in the folder name must not be expanded.
        let directory = parent.path().join("100%(id)s");
        let progress = Progress::default();
        let saved = download(
            &backend,
            ffmpeg.as_deref(),
            &info,
            format,
            &directory,
            &progress,
        )
        .await
        .unwrap();
        assert!(saved.bytes > 1000);
        assert!(saved.path.starts_with(directory.to_str().unwrap()));
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    }
}
