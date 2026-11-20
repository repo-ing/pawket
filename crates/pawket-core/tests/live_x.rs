//! Live checks against real public X posts. They need network access and X's
//! cooperation, so they are opt-in:
//!
//!     cargo test -p pawket-core --test live_x -- --ignored --test-threads=1
//!
//! The offline fixtures in `tests/fixtures/x/` cover parsing; these confirm
//! the endpoint, token and media hosts still behave the same way.

use pawket_core::{Engine, MediaKind};

const VIDEO_WITH_SOUND: &str = "https://x.com/elonmusk/status/1585341984679469056";
const SILENT_VIDEO: &str = "https://twitter.com/NASAUniverse/status/1505976485538111504";
const GIF: &str = "https://x.com/morecats4/status/2088592754830184499";
const PHOTO: &str = "https://x.com/NASAWebb/status/1546621080298835970";
const PROTECTED: &str = "https://x.com/Support/status/479307198901026816";

#[tokio::test]
#[ignore = "live network"]
async fn video_post_returns_mp4_variants_with_sound() {
    let info = Engine::new()
        .unwrap()
        .inspect(VIDEO_WITH_SOUND)
        .await
        .unwrap();
    assert_eq!(info.provider, "x");
    assert!(info.formats.len() >= 2);
    assert_eq!(info.formats[0].id, "x-1-1920x1080");
    assert!(info.formats.iter().all(|f| f.has_audio == Some(true)));
    assert!(info.unsupported.iter().any(|u| u.kind == "hls"));
}

#[tokio::test]
#[ignore = "live network"]
async fn silent_video_and_gif_are_video_only() {
    let engine = Engine::new().unwrap();
    for url in [SILENT_VIDEO, GIF] {
        let info = engine.inspect(url).await.unwrap();
        assert!(
            info.formats.iter().all(|f| f.has_audio == Some(false)),
            "{url}"
        );
    }
}

#[tokio::test]
#[ignore = "live network"]
async fn photo_and_download_preserve_original_bytes() {
    let engine = Engine::new().unwrap();
    let info = engine.inspect(PHOTO).await.unwrap();
    assert_eq!(info.formats[0].kind, MediaKind::Image);
    let directory = tempfile::tempdir().unwrap();
    let saved = engine
        .download(GIF, directory.path(), Some("x-1-gif"))
        .await
        .unwrap();
    let bytes = std::fs::read(&saved.path).unwrap();
    assert_eq!(bytes.len() as u64, saved.bytes);
    assert_eq!(&bytes[4..8], b"ftyp", "saved file is an MP4 container");
}

#[tokio::test]
#[ignore = "live network"]
async fn protected_and_missing_posts_fail_clearly() {
    let engine = Engine::new().unwrap();
    let error = engine.inspect(PROTECTED).await.unwrap_err();
    assert_eq!(error.code(), "source_unavailable");
    let error = engine
        .inspect("https://x.com/a/status/1")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "source_unavailable");
}
