use super::{Backend, GuardProxy, Run, DOWNLOAD_TIMEOUT};
use crate::{PawketError, Progress, Result};
use serde::Deserialize;
use std::{path::Path, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{ChildStdin, ChildStdout},
    sync::Mutex,
    time::timeout,
};

#[derive(Default)]
pub(super) struct AudioWorker(Mutex<Option<Session>>);

impl std::fmt::Debug for AudioWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AudioWorker")
    }
}

struct Session {
    child: Run,
    _proxy: GuardProxy,
    _plugin: tempfile::TempDir,
    stdin: ChildStdin,
    replies: Lines<BufReader<ChildStdout>>,
}

#[derive(Deserialize)]
struct Reply {
    done: Option<bool>,
    written: Option<u64>,
    total: Option<u64>,
}

impl Session {
    async fn start(backend: &Backend) -> Result<Self> {
        let plugin = tempfile::tempdir()?;
        let package = plugin.path().join("pawket/yt_dlp_plugins/extractor");
        std::fs::create_dir_all(&package)?;
        std::fs::write(package.join("audio.py"), include_bytes!("audio_worker.py"))?;
        let proxy = GuardProxy::start().await?;
        let mut command = backend.command(&proxy);
        command
            .arg("--plugin-dirs")
            .arg(plugin.path())
            .args([
                "--quiet",
                "--no-progress",
                "--no-simulate",
                "--no-mtime",
                "-f",
                "ba",
                "--match-filter",
                "live_status!=?is_live & live_status!=?is_upcoming",
                "--extractor-args",
                "youtube:player_client=tv_samsung;skip=hls,dash",
                "-o",
                super::OUTPUT_TEMPLATE,
                "--",
                "pawket-audio:worker",
            ])
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = Run::spawn(command)?;
        let stdin = child.stdin()?;
        let replies = BufReader::new(child.stdout()?).lines();
        Ok(Self {
            child,
            _proxy: proxy,
            _plugin: plugin,
            stdin,
            replies,
        })
    }

    async fn download(&mut self, id: &str, directory: &Path, progress: &Progress) -> Result<bool> {
        let mut line = serde_json::to_vec(&serde_json::json!({"id":id,"directory":directory}))
            .map_err(|_| PawketError::InvalidUrl)?;
        line.push(b'\n');
        self.stdin.write_all(&line).await?;
        self.stdin.flush().await?;
        while let Some(line) = self.replies.next_line().await? {
            let reply: Reply =
                serde_json::from_str(&line).map_err(|_| PawketError::BackendUnavailable)?;
            if let Some(ok) = reply.done {
                return Ok(ok);
            }
            if let Some(written) = reply.written {
                progress.set_written(written);
                progress.set_total(reply.total);
            }
        }
        Err(PawketError::BackendUnavailable)
    }
}

impl AudioWorker {
    pub(super) async fn warm(&self, backend: &Backend) {
        let mut slot = self.0.lock().await;
        if slot.is_none() {
            *slot = Session::start(backend).await.ok();
        }
    }

    pub(super) async fn download(
        &self,
        backend: &Backend,
        id: &str,
        directory: &Path,
        progress: &Progress,
    ) -> bool {
        // A second simultaneous download can use the ordinary subprocess.
        let Ok(mut slot) = self.0.try_lock() else {
            return false;
        };
        let mut session = match slot
            .take()
            .and_then(|mut session| session.child.is_running().then_some(session))
        {
            Some(session) => session,
            None => match Session::start(backend).await {
                Ok(s) => s,
                Err(_) => return false,
            },
        };
        // Taking ownership also drops the whole process group on cancellation.
        if let Ok(Ok(success)) =
            timeout(DOWNLOAD_TIMEOUT, session.download(id, directory, progress)).await
        {
            *slot = Some(session);
            return success;
        }
        false
    }
}
