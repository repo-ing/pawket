//! Finds optional helper programs (FFmpeg, yt-dlp) on PATH, so `pawket serve`
//! works without flags when they are installed.

use std::path::{Path, PathBuf};

/// The first executable called `name` in `dirs`.
pub(crate) fn find_in<I>(dirs: I, name: &str) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    dirs.into_iter()
        .map(|dir| dir.join(&file))
        .find(|candidate| is_executable(candidate))
}

/// The first executable called `name` on PATH.
pub(crate) fn find(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_in(std::env::split_paths(&path), name)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_first_executable_and_skips_plain_files() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        };
        std::fs::write(first.path().join(name), b"not executable").unwrap();
        let wanted = second.path().join(name);
        std::fs::write(&wanted, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wanted, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dirs = || vec![first.path().to_path_buf(), second.path().to_path_buf()];
        if cfg!(unix) {
            assert_eq!(find_in(dirs(), "yt-dlp"), Some(wanted));
        }
        assert_eq!(find_in(dirs(), "ffmpeg"), None);
        assert_eq!(find_in(Vec::new(), "yt-dlp"), None);
    }
}
