//! One yt-dlp process: its own process group, bounded output, and cleanup
//! of the whole group (including FFmpeg children) on every early exit.

use crate::{PawketError, Result};
use std::{process::ExitStatus, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, ChildStderr, ChildStdout, Command},
    time::timeout,
};

const READ_CHUNK: usize = 8 * 1024;

pub(crate) struct Run {
    child: Child,
    /// Group to stop on drop; cleared once the process has been reaped.
    group: Option<u32>,
}

impl Run {
    pub(crate) fn spawn(mut command: Command) -> Result<Self> {
        #[cfg(unix)]
        command.process_group(0);
        let child = command
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| PawketError::BackendUnavailable)?;
        let group = child.id();
        Ok(Self { child, group })
    }

    pub(crate) fn stdout(&mut self) -> Result<ChildStdout> {
        self.child
            .stdout
            .take()
            .ok_or(PawketError::BackendUnavailable)
    }

    pub(crate) fn stderr(&mut self) -> Result<ChildStderr> {
        self.child
            .stderr
            .take()
            .ok_or(PawketError::BackendUnavailable)
    }

    /// Wait for a process whose output has closed. A process that lingers
    /// past `limit` is stopped (with its group) when `self` is dropped.
    pub(crate) async fn finish(mut self, limit: Duration) -> Result<ExitStatus> {
        let status = timeout(limit, self.child.wait())
            .await
            .map_err(|_| PawketError::Network("yt-dlp didn’t exit".into()))?
            .map_err(|_| PawketError::BackendUnavailable)?;
        // Reaped: the group ID may be reused from now on, so never signal it.
        self.group = None;
        Ok(status)
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(group) = self.group.take() {
            stop_group(group);
        }
    }
}

#[cfg(unix)]
fn stop_group(group: u32) {
    let Ok(group) = libc::pid_t::try_from(group) else {
        return;
    };
    // SAFETY: killpg only sends a signal. `group` is the ID of a group this
    // Run created with process_group(0) and whose leader has not been reaped
    // yet (finish() clears it after reaping), so the ID cannot refer to an
    // unrelated, reused group.
    unsafe {
        libc::killpg(group, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn stop_group(_group: u32) {
    // kill_on_drop stops yt-dlp itself; child processes are left to exit.
}

/// Read a stream to its end, keeping at most `limit` bytes. The flag is true
/// when bytes were dropped. Draining keeps the writer from blocking or
/// failing on a closed pipe.
pub(crate) async fn read_capped<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut overflowed = false;
    let mut chunk = vec![0u8; READ_CHUNK];
    while let Ok(read) = reader.read(&mut chunk).await {
        if read == 0 {
            break;
        }
        let room = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..read.min(room)]);
        overflowed |= read > room;
    }
    (kept, overflowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_capped_keeps_the_start_and_reports_overflow() {
        let (kept, overflowed) = read_capped(&b"abcdef"[..], 4).await;
        assert_eq!((kept.as_slice(), overflowed), (&b"abcd"[..], true));
        let (kept, overflowed) = read_capped(&b"abc"[..], 4).await;
        assert_eq!((kept.as_slice(), overflowed), (&b"abc"[..], false));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_a_run_stops_its_child_processes() {
        let marker = tempfile::NamedTempFile::new().unwrap();
        let mut command = Command::new("/bin/sh");
        // The shell starts a grandchild that would outlive a plain kill.
        command.arg("-c").arg(format!(
            "(sleep 30; echo late > {}) & wait",
            marker.path().display()
        ));
        let run = Run::spawn(command).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(run);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let survivors = std::process::Command::new("/usr/bin/pgrep")
            .args(["-f", &format!("echo late > {}", marker.path().display())])
            .output()
            .unwrap();
        assert!(survivors.stdout.is_empty(), "grandchild survived");
    }
}
