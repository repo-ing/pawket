use crate::{PawketError, Result};
use std::path::{Path, PathBuf};
use tempfile::TempPath;

pub fn safe_basename(title: &str) -> String {
    let mut name: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(100)
        .collect();
    name = name.trim_matches('-').to_string();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    if name.is_empty() {
        name = "media".into();
    }
    // All final names also receive an explicit extension. Prefix reserved DOS
    // names so the same downloads work when a source tree is used on Windows.
    let upper = name.to_ascii_uppercase();
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&upper.as_str())
    {
        name.insert_str(0, "media-");
    }
    name
}

pub async fn create_temporary(directory: &Path) -> Result<(tokio::fs::File, TempPath)> {
    tokio::fs::create_dir_all(directory).await?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".pawket-part-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The committed hard link retains the usual output-file permissions.
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    let temporary = builder.tempfile_in(directory)?;
    let (file, path) = temporary.into_parts();
    Ok((tokio::fs::File::from_std(file), path))
}

pub async fn commit_temporary(
    temp: &Path,
    directory: &Path,
    title: &str,
    extension: &str,
) -> Result<PathBuf> {
    let name = safe_basename(title);
    let extension: String = extension
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if extension.is_empty() {
        return Err(PawketError::NotMedia);
    }
    for count in 0..1000 {
        let suffix = if count == 0 {
            String::new()
        } else {
            format!("-{count}")
        };
        let destination = directory.join(format!("{name}{suffix}.{extension}"));
        // hard_link atomically fails if the destination already exists; unlike
        // rename, it cannot clobber an existing download in a race.
        match tokio::fs::hard_link(temp, &destination).await {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(PawketError::Storage(
        "Too many downloads already have this filename".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_cannot_escape_download_directory() {
        for title in [
            "../../secret",
            "C:\\Windows\\system32",
            "/etc/passwd",
            "🪲",
            "\0\r\n",
            "CON",
            "...",
        ] {
            let name = safe_basename(title);
            assert!(!name.contains('/') && !name.contains('\\'));
            assert!(!name.contains('.') && !name.is_empty());
            assert_ne!(name, "CON");
        }
    }

    #[tokio::test]
    async fn commits_without_clobbering_and_keeps_bytes() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let (mut file, temp) = create_temporary(dir.path()).await.unwrap();
        file.write_all(b"new-media").await.unwrap();
        file.sync_all().await.unwrap();
        drop(file);
        tokio::fs::write(dir.path().join("clip.mp4"), b"original")
            .await
            .unwrap();
        let committed = commit_temporary(&temp, dir.path(), "clip", "mp4")
            .await
            .unwrap();
        assert_eq!(committed.file_name().unwrap(), "clip-1.mp4");
        assert_eq!(
            tokio::fs::read(dir.path().join("clip.mp4")).await.unwrap(),
            b"original"
        );
        assert_eq!(tokio::fs::read(committed).await.unwrap(), b"new-media");
    }

    #[tokio::test]
    async fn unique_temporary_files_do_not_overlap() {
        let dir = tempfile::tempdir().unwrap();
        let (_, first) = create_temporary(dir.path()).await.unwrap();
        let (_, second) = create_temporary(dir.path()).await.unwrap();
        assert_ne!(&*first, &*second);
    }
}
