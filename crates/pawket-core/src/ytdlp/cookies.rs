//! Signed-in access: hand yt-dlp the operator's browser session or a
//! cookies file, so private, members-only and login-gated media work.

use crate::{PawketError, Result};
use std::path::PathBuf;

/// Browsers yt-dlp can read cookies from.
const BROWSERS: [&str; 9] = [
    "brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi", "whale",
];
const MAX_SPEC_CHARS: usize = 512;

/// Where yt-dlp gets login cookies from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CookieSource {
    /// yt-dlp's `BROWSER[+KEYRING][:PROFILE][::CONTAINER]` form, e.g. `chrome`
    /// or `firefox:default-release`.
    Browser(String),
    /// A Netscape-format cookies file. yt-dlp writes refreshed cookies back.
    File(PathBuf),
}

impl CookieSource {
    /// Check a `--cookies-from-browser` value before any run uses it.
    pub fn browser(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        let name_end = spec.find(['+', ':']).unwrap_or(spec.len());
        let name = spec[..name_end].to_ascii_lowercase();
        let valid = spec.len() <= MAX_SPEC_CHARS
            && !spec.chars().any(char::is_control)
            && BROWSERS.contains(&name.as_str());
        if !valid {
            return Err(PawketError::CookiesUnavailable(format!(
                "Use one of {} for --cookies-from-browser.",
                BROWSERS.join(", ")
            )));
        }
        Ok(Self::Browser(format!("{name}{}", &spec[name_end..])))
    }

    /// Check a `--cookies` file exists and resolve it to an absolute path.
    pub async fn file(path: PathBuf) -> Result<Self> {
        let path = tokio::fs::canonicalize(&path).await.map_err(|_| {
            PawketError::CookiesUnavailable("The cookies file wasn’t found.".into())
        })?;
        let metadata = tokio::fs::metadata(&path).await?;
        if !metadata.is_file() {
            return Err(PawketError::CookiesUnavailable(
                "The cookies path isn’t a file.".into(),
            ));
        }
        Ok(Self::File(path))
    }

    /// Short description for capability reports: the browser name or "file".
    pub fn kind(&self) -> String {
        match self {
            Self::Browser(spec) => spec
                .split(['+', ':'])
                .next()
                .unwrap_or_default()
                .to_string(),
            Self::File(_) => "file".into(),
        }
    }

    pub(crate) fn args(&self) -> Vec<std::ffi::OsString> {
        match self {
            Self::Browser(spec) => vec!["--cookies-from-browser".into(), spec.into()],
            Self::File(path) => vec!["--cookies".into(), path.into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_browsers_with_profiles_and_rejects_the_rest() {
        assert_eq!(
            CookieSource::browser("Chrome").unwrap(),
            CookieSource::Browser("chrome".into())
        );
        assert_eq!(
            CookieSource::browser("firefox:default-release").unwrap(),
            CookieSource::Browser("firefox:default-release".into())
        );
        assert!(CookieSource::browser("chromium+basictext:/tmp/Profile 1").is_ok());
        for bad in ["", "netscape", "--exec", "chrome\nx", "ego"] {
            assert!(CookieSource::browser(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(CookieSource::browser("safari").unwrap().kind(), "safari");
    }

    #[tokio::test]
    async fn cookie_files_must_exist_and_be_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(CookieSource::file(dir.path().join("missing.txt"))
            .await
            .is_err());
        assert!(CookieSource::file(dir.path().to_path_buf()).await.is_err());
        let file = dir.path().join("cookies.txt");
        std::fs::write(&file, "# Netscape HTTP Cookie File\n").unwrap();
        let source = CookieSource::file(file).await.unwrap();
        assert_eq!(source.kind(), "file");
        assert_eq!(source.args()[0], "--cookies");
    }
}
