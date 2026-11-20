//! Recognises well-known sites that this version does not extract, so users
//! get a clear "not supported yet" instead of a generic HTML failure.

use url::Url;

/// Hosts with no adapter yet, mapped to the name shown to users.
const UNSUPPORTED: [(&str, &str); 6] = [
    ("youtube.com", "YouTube"),
    ("youtu.be", "YouTube"),
    ("youtube-nocookie.com", "YouTube"),
    ("tiktok.com", "TikTok"),
    ("x.com", "X profile, search and non-post"),
    ("twitter.com", "X profile, search and non-post"),
];

/// True when `host` is `domain` or one of its subdomains.
pub fn host_matches(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// Name of the unsupported provider for this URL, if it is one.
/// Call after the X post matcher: X *posts* are supported.
pub fn unsupported_provider(url: &Url) -> Option<&'static str> {
    let host = url.host_str()?;
    UNSUPPORTED
        .iter()
        .find(|(domain, _)| host_matches(host, domain))
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(input: &str) -> Option<&'static str> {
        unsupported_provider(&Url::parse(input).unwrap())
    }

    #[test]
    fn names_unsupported_social_sites() {
        assert_eq!(
            provider("https://www.youtube.com/watch?v=aqz-KE-bpKQ"),
            Some("YouTube")
        );
        assert_eq!(provider("https://youtu.be/aqz-KE-bpKQ"), Some("YouTube"));
        assert_eq!(provider("https://m.youtube.com/shorts/x"), Some("YouTube"));
        assert_eq!(
            provider("https://www.tiktok.com/@a/video/1"),
            Some("TikTok")
        );
        assert_eq!(provider("https://vm.tiktok.com/abc/"), Some("TikTok"));
        assert!(provider("https://x.com/NASA").is_some());
    }

    #[test]
    fn does_not_match_lookalike_domains() {
        assert_eq!(provider("https://notyoutube.com/watch"), None);
        assert_eq!(provider("https://youtube.com.evil.example/watch"), None);
        assert_eq!(provider("https://upload.wikimedia.org/a.webm"), None);
    }
}
