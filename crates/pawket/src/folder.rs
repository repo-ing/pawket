use std::{
    ffi::OsStr,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::NamedTempFile;

#[derive(Clone)]
pub(crate) struct Folder {
    setting: PathBuf,
}

impl Folder {
    pub fn new(output: &Path) -> Self {
        Self {
            setting: output.join("save-folder.json"),
        }
    }

    pub fn path(&self) -> io::Result<PathBuf> {
        match fs::read(&self.setting) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                    .ok_or_else(|| io::Error::other("Choose a folder to save your files."))?;
                Ok(PathBuf::from(home).join("Downloads").join("Pawket"))
            }
            Err(e) => Err(e),
        }
    }

    pub fn destination(&self) -> io::Result<PathBuf> {
        let path = self.path()?;
        if !self.setting.try_exists()? {
            fs::create_dir_all(&path)?;
        }
        if !path.is_dir() {
            return Err(io::Error::other(
                "Your Pawket folder is unavailable. Reconnect it or choose another folder.",
            ));
        }
        Ok(path)
    }

    pub fn choose(&self) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        let output = Command::new("osascript")
            .args([
                "-e",
                "POSIX path of (choose folder with prompt \"Choose your Pawket save folder\")",
            ])
            .output()?;
        #[cfg(windows)]
        let output = Command::new("powershell.exe").args(["-NoProfile","-STA","-Command","Add-Type -AssemblyName System.Windows.Forms; $p = New-Object System.Windows.Forms.FolderBrowserDialog; $p.Description='Choose your Pawket save folder'; [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); if ($p.ShowDialog() -eq 'OK') { [Console]::Write($p.SelectedPath) }"]).output()?;
        #[cfg(not(any(windows, target_os = "macos")))]
        let output = Command::new("zenity")
            .args([
                "--file-selection",
                "--directory",
                "--title=Choose your Pawket save folder",
            ])
            .output()?;
        if !output.status.success() {
            if output.status.code() == Some(1)
                && (output.stderr.is_empty()
                    || String::from_utf8_lossy(&output.stderr).contains("(-128)"))
            {
                return Ok(());
            }
            return Err(io::Error::other(
                "Could not open the system folder chooser.",
            ));
        }
        let raw = String::from_utf8(output.stdout).map_err(io::Error::other)?;
        let name = raw.trim_end_matches(['\r', '\n']);
        if name.is_empty() {
            return Ok(());
        }
        self.remember(Path::new(name))
    }

    fn remember(&self, path: &Path) -> io::Result<()> {
        if !path.is_absolute() || !path.is_dir() {
            return Err(io::Error::other("Select an available folder."));
        }
        let mut tmp = NamedTempFile::new_in(self.setting.parent().unwrap())?;
        serde_json::to_writer(&mut tmp, &path)?;
        tmp.flush()?;
        tmp.persist(&self.setting)?;
        Ok(())
    }

    pub fn open(&self) -> io::Result<()> {
        let program = if cfg!(windows) {
            "explorer.exe"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        Command::new(program).arg(self.destination()?).spawn()?;
        Ok(())
    }
}

pub(crate) fn keep(source: &Path, directory: &Path, filename: &str) -> io::Result<String> {
    let name = Path::new(filename);
    if name.file_name() != Some(OsStr::new(filename)) || filename.is_empty() {
        return Err(io::Error::other("Invalid media filename"));
    }
    let mut tmp = NamedTempFile::new_in(directory)?;
    io::copy(&mut fs::File::open(source)?, tmp.as_file_mut())?;
    tmp.as_file().sync_all()?;
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let suffix = name
        .extension()
        .map(|x| format!(".{}", x.to_string_lossy()))
        .unwrap_or_default();
    for number in 1u32.. {
        let candidate = if number == 1 {
            filename.to_owned()
        } else {
            format!("{stem} ({number}){suffix}")
        };
        match tmp.persist_noclobber(directory.join(&candidate)) {
            Ok(_) => return Ok(candidate),
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => tmp = e.file,
            Err(e) => return Err(e.error),
        }
    }
    Err(io::Error::other("Too many files with this name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_survive_restart_and_colliding_names() -> io::Result<()> {
        let config = tempfile::tempdir()?;
        let library = tempfile::tempdir()?;
        Folder::new(config.path()).remember(library.path())?;
        let source = config.path().join("input");
        fs::write(&source, b"first")?;
        let path = Folder::new(config.path()).destination()?;
        assert_eq!(keep(&source, &path, "clip.mp4")?, "clip.mp4");
        fs::write(&source, b"second")?;
        assert_eq!(keep(&source, &path, "clip.mp4")?, "clip (2).mp4");
        assert_eq!(fs::read(path.join("clip.mp4"))?, b"first");
        assert_eq!(fs::read(path.join("clip (2).mp4"))?, b"second");
        drop(library);
        assert!(Folder::new(config.path()).destination().is_err());
        Ok(())
    }
}
