use super::{
    hls::{hls_attribute, playlist_has_audio},
    token::{syndication_token, to_radix36},
    *,
};

fn id(input: &str) -> Option<String> {
    post_id(&Url::parse(input).unwrap())
}

fn fixture(name: &str) -> Post {
    let path = format!(
        "{}/tests/fixtures/x/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn parse(name: &str, post_id: &str) -> Result<MediaInfo> {
    let source = Url::parse(&format!("https://x.com/i/status/{post_id}")).unwrap();
    parse_post(&source, post_id, fixture(name))
}

#[test]
fn recognises_post_links_on_both_domains() {
    let expected = Some("1585341984679469056".to_owned());
    for input in [
        "https://x.com/elonmusk/status/1585341984679469056",
        "https://twitter.com/elonmusk/status/1585341984679469056?s=20",
        "https://mobile.twitter.com/elonmusk/status/1585341984679469056",
        "https://www.x.com/elonmusk/status/1585341984679469056/video/1",
        "https://x.com/i/status/1585341984679469056",
        "https://x.com/i/web/status/1585341984679469056",
    ] {
        assert_eq!(id(input), expected, "{input}");
    }
}

#[test]
fn rejects_non_post_and_lookalike_links() {
    for input in [
        "https://x.com/elonmusk",
        "https://x.com/elonmusk/status/",
        "https://x.com/elonmusk/status/12ab",
        "https://x.com/a/status/123456789012345678901234",
        "https://notx.com/a/status/1",
        "https://x.com.evil.example/a/status/1",
        "https://example.com/a/status/1",
    ] {
        assert_eq!(id(input), None, "{input}");
    }
}

#[test]
fn token_matches_javascript_radix_formatting() {
    // Expected strings were produced by JavaScript (V8) on 2026-11-06.
    assert_eq!(to_radix36(0.5), "0.i");
    assert_eq!(to_radix36(35.0), "z");
    assert_eq!(to_radix36(36.0), "10");
    assert_eq!(to_radix36(0.1), "0.3lllllllllm");
    assert_eq!(to_radix36(std::f64::consts::PI), "3.53i5ab8p5f");
    assert_eq!(syndication_token("20"), "6dq1a2xwd93");
    assert_eq!(syndication_token("1585341984679469056"), "3uchycv2wqc");
    assert_eq!(syndication_token("2088592754830184499"), "529i9x5bgkk");
    assert_eq!(syndication_token("479307198901026816"), "15tsd7r7sczh");
}

#[test]
fn video_post_lists_mp4_variants_best_first_and_labels_hls() {
    let info = parse("video", "1585341984679469056").unwrap();
    assert_eq!(info.provider, "x");
    assert_eq!(
        info.source_url,
        "https://x.com/elonmusk/status/1585341984679469056"
    );
    assert_eq!(
        info.title,
        "@elonmusk – Entering Twitter HQ – let that sink in!"
    );
    let ids: Vec<&str> = info.formats.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "x-1-1920x1080",
            "x-1-1280x720",
            "x-1-640x360",
            "x-1-480x270"
        ]
    );
    let best = &info.formats[0];
    assert_eq!(
        (best.width, best.height, best.bitrate),
        (Some(1920), Some(1080), Some(10_368_000))
    );
    assert_eq!(best.duration_ms, Some(9301));
    assert_eq!(best.has_audio, None);
    assert_eq!(best.label.as_deref(), Some("1920×1080"));
    assert!(best.url.starts_with("https://video.twimg.com/"));
    assert!(info
        .formats
        .iter()
        .all(|f| f.extension == "mp4" && f.kind == MediaKind::Video));
    assert_eq!(info.unsupported.len(), 1);
    assert_eq!(info.unsupported[0].kind, "hls");
}

#[test]
fn amplify_video_and_codec_paths_are_parsed() {
    let nasa = parse("video-amplify", "1505976485538111504").unwrap();
    assert_eq!(nasa.formats.len(), 3);
    assert_eq!(nasa.formats[0].id, "x-1-1280x720");
    let silent = parse("video-short", "1779732036124557398").unwrap();
    assert_eq!(silent.formats.len(), 1);
    assert_eq!(silent.formats[0].id, "x-1-264x258");
}

#[test]
fn gif_is_labelled_video_only() {
    let info = parse("gif", "2088592754830184499").unwrap();
    assert_eq!(info.formats.len(), 1);
    let gif = &info.formats[0];
    assert_eq!(gif.id, "x-1-gif");
    assert_eq!(gif.has_audio, Some(false));
    assert_eq!(gif.bitrate, None);
    assert_eq!((gif.width, gif.height), (Some(320), Some(320)));
    assert!(gif.label.as_deref().unwrap().contains("no sound"));
    assert!(info.unsupported.is_empty());
}

#[test]
fn photo_requests_the_original_size() {
    let info = parse("photo", "1546621080298835970").unwrap();
    let photo = &info.formats[0];
    assert_eq!(photo.id, "x-1-photo");
    assert_eq!(photo.kind, MediaKind::Image);
    assert_eq!(
        photo.url,
        "https://pbs.twimg.com/media/FXazrAmWAAEP0Xe.jpg?name=orig"
    );
    assert_eq!(photo.extension, "jpg");
}

#[test]
fn protected_and_text_only_posts_give_clear_errors() {
    let error = parse("tombstone", "479307198901026816").unwrap_err();
    assert_eq!(error.code(), "source_unavailable");
    assert!(error.to_string().contains("limits who can view"));
    let error = parse("text-only", "20").unwrap_err();
    assert_eq!(
        error.to_string(),
        "That X post has no video, GIF or photo to save."
    );
    let error = parse_post(
        &Url::parse("https://x.com/i/status/1").unwrap(),
        "1",
        Post::default(),
    )
    .unwrap_err();
    assert_eq!(error.code(), "source_unavailable");
}

#[test]
fn hls_only_video_is_reported_not_hidden() {
    let json = r#"{"__typename":"Tweet","text":"clip","user":{"screen_name":"a"},
            "mediaDetails":[{"type":"video","video_info":{"variants":[
              {"content_type":"application/x-mpegURL","url":"https://video.twimg.com/a.m3u8"}]}}]}"#;
    let post: Post = serde_json::from_str(json).unwrap();
    let error =
        parse_post(&Url::parse("https://x.com/a/status/5").unwrap(), "5", post).unwrap_err();
    assert!(error.to_string().contains("only available as a stream"));
}

#[test]
fn variant_urls_still_pass_the_network_policy() {
    let json = r#"{"__typename":"Tweet","text":"x","user":{"screen_name":"a"},
            "mediaDetails":[{"type":"video","video_info":{"variants":[
              {"bitrate":1,"content_type":"video/mp4","url":"https://127.0.0.1/vid/1x1/a.mp4"},
              {"bitrate":2,"content_type":"video/mp4","url":"http://video.twimg.com/vid/2x2/a.mp4"}]}}]}"#;
    let post: Post = serde_json::from_str(json).unwrap();
    assert!(parse_post(&Url::parse("https://x.com/a/status/5").unwrap(), "5", post).is_err());
}

fn playlist(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/x/{name}.m3u8",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn playlists_reveal_whether_a_video_has_sound() {
    assert_eq!(playlist_has_audio(&playlist("video")), Some(true));
    assert_eq!(playlist_has_audio(&playlist("video-amplify")), Some(false));
    assert_eq!(playlist_has_audio(&playlist("video-short")), Some(false));
    assert_eq!(playlist_has_audio("<html>not a playlist</html>"), None);
    assert_eq!(playlist_has_audio("#EXTM3U\n#EXT-X-VERSION:6\n"), None);
    let muxed = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,CODECS=\"avc1.4D401E,mp4a.40.2\"\na.m3u8\n";
    assert_eq!(playlist_has_audio(muxed), Some(true));
}

#[test]
fn silent_videos_are_labelled_video_only() {
    let info = parse("video-amplify", "1505976485538111504").unwrap();
    let info = with_audio(info, 1, false);
    assert!(info.formats.iter().all(|f| f.has_audio == Some(false)));
    assert_eq!(
        info.formats[0].label.as_deref(),
        Some("1280×720 · no sound")
    );
    let with_sound = with_audio(parse("video", "1585341984679469056").unwrap(), 1, true);
    assert_eq!(with_sound.formats[0].has_audio, Some(true));
    assert_eq!(with_sound.formats[0].label.as_deref(), Some("1920×1080"));
    let gif = with_audio(parse("gif", "2088592754830184499").unwrap(), 1, true);
    assert_eq!(
        gif.formats[0].has_audio,
        Some(false),
        "a stated GIF state is not overwritten"
    );
}

#[test]
fn titles_decode_x_html_escapes() {
    let info = parse("photo", "1546621080298835970").unwrap();
    assert!(info.title.contains("deepest & sharpest"), "{}", info.title);
    assert_eq!(decode_entities("a &amp;lt; b &lt; c"), "a &lt; b < c");
}

fn post(json: &str) -> Result<MediaInfo> {
    let post: Post = serde_json::from_str(json).unwrap();
    parse_post(&Url::parse("https://x.com/i/status/5").unwrap(), "5", post)
}

#[test]
fn media_must_come_from_x_cdn() {
    let error = post(r#"{"__typename":"Tweet","user":{"screen_name":"a"},"mediaDetails":[
            {"type":"photo","media_url_https":"https://evil.example/media/a.jpg"},
            {"type":"video","video_info":{"variants":[{"bitrate":1,"content_type":"video/mp4","url":"https://evil.example/vid/1x1/a.mp4"}]}}]}"#)
            .unwrap_err();
    assert!(
        error.to_string().contains("couldn’t read file 1"),
        "{error}"
    );
    assert!(
        error.to_string().contains("couldn’t read file 2"),
        "{error}"
    );
}

#[test]
fn one_malformed_entry_does_not_hide_the_rest() {
    let info = post(
        r#"{"__typename":"Tweet","user":{"screen_name":"a"},"mediaDetails":[
            {"type":"photo","original_info":{"width":"oops"}},
            {"type":"photo","media_url_https":"https://pbs.twimg.com/media/b.png"}]}"#,
    )
    .unwrap();
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].id, "x-2-photo");
    assert_eq!(info.unsupported[0].kind, "unreadable");
    let error = post(r#"{"__typename":"Tweet","mediaDetails":null}"#).unwrap_err();
    assert_eq!(
        error.to_string(),
        "That X post has no video, GIF or photo to save."
    );
}

#[test]
fn invalid_screen_names_are_not_used_in_urls_or_titles() {
    let info = post(
        r#"{"__typename":"Tweet","text":"hi","user":{"screen_name":"a/../b?x"},"mediaDetails":[
            {"type":"photo","media_url_https":"https://pbs.twimg.com/media/b.jpg"}]}"#,
    )
    .unwrap();
    assert_eq!(info.source_url, "https://x.com/i/status/5");
    assert_eq!(info.title, "hi");
}

#[test]
fn renditions_without_sizes_keep_distinct_ids() {
    let info = post(r#"{"__typename":"Tweet","user":{"screen_name":"a"},"mediaDetails":[{"type":"video",
            "original_info":{"width":1920,"height":1080},"video_info":{"variants":[
            {"bitrate":2000,"content_type":"video/mp4","url":"https://video.twimg.com/v/hi.mp4"},
            {"bitrate":1000,"content_type":"video/mp4","url":"https://video.twimg.com/v/lo.mp4"}]}}]}"#)
            .unwrap();
    let ids: Vec<&str> = info.formats.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["x-1-2000", "x-1-1000"]);
    assert!(info.formats.iter().all(|f| f.width.is_none()));
}

#[test]
fn audio_detection_needs_complete_evidence() {
    let partial = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,CODECS=\"avc1.4D401E\"\na.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2\nb.m3u8\n";
    assert_eq!(playlist_has_audio(partial), None);
    let supplemental = "#EXTM3U\n#EXT-X-STREAM-INF:SUPPLEMENTAL-CODECS=\"mp4a.40.2\",CODECS=\"avc1.4D401E\"\na.m3u8\n";
    assert_eq!(playlist_has_audio(supplemental), Some(false));
    let uri_only = "#EXTM3U\n#EXT-X-STREAM-INF:CODECS=\"avc1.4D401E\"\nTYPE=AUDIO.m3u8\n";
    assert_eq!(playlist_has_audio(uri_only), Some(false));
    assert_eq!(hls_attribute(r#"A=1,B="x,y",C=z"#, "C"), Some("z"));
    assert_eq!(hls_attribute(r#"A=1,B="x,y",C=z"#, "B"), Some("x,y"));
}

#[test]
fn formats_carry_their_media_item_number() {
    let info = post(
        r#"{"__typename":"Tweet","user":{"screen_name":"a"},"mediaDetails":[
            {"type":"photo","media_url_https":"https://pbs.twimg.com/media/a.jpg"},
            {"type":"video","video_info":{"variants":[
              {"bitrate":2,"content_type":"video/mp4","url":"https://video.twimg.com/vid/4x4/a.mp4"},
              {"bitrate":1,"content_type":"video/mp4","url":"https://video.twimg.com/vid/2x2/a.mp4"}]}}]}"#,
    )
    .unwrap();
    let items: Vec<Option<u32>> = info.formats.iter().map(|f| f.item).collect();
    assert_eq!(items, [Some(1), Some(2), Some(2)]);
    let labels: Vec<&str> = info
        .formats
        .iter()
        .filter_map(|f| f.label.as_deref())
        .collect();
    assert_eq!(labels, ["Photo 1", "Video 2 · 4×4", "Video 2 · 2×2"]);
}
