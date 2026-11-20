//! Public routes for sites whose main pages now need a login in yt-dlp,
//! but whose own embeddable players still serve anonymous visitors.

use crate::providers::host_matches;
use url::Url;

const VIMEO_REFERER: &str = "https://vimeo.com/";
/// Vimeo IDs are numeric; unlisted videos add a short hex hash.
const MIN_VIMEO_ID_DIGITS: usize = 5;
const MAX_VIMEO_HASH_CHARS: usize = 20;
/// Vimeo paths that list several videos rather than one.
const VIMEO_COLLECTIONS: [&str; 5] = ["showcase", "album", "user", "ondemand", "upload"];

/// The link to give yt-dlp, plus any extra arguments that route needs.
pub(crate) fn public_route(url: &Url) -> (Url, Vec<String>) {
    if let Some(player) = vimeo_player(url) {
        return (player, vec!["--referer".into(), VIMEO_REFERER.into()]);
    }
    if is_vimeo_player(url) {
        return (url.clone(), vec!["--referer".into(), VIMEO_REFERER.into()]);
    }
    (url.clone(), Vec::new())
}

fn is_vimeo_player(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("player.vimeo.com"))
}

/// `vimeo.com/<id>[/<hash>]` (also under channels and groups) becomes the
/// public player link `player.vimeo.com/video/<id>[?h=<hash>]`.
fn vimeo_player(url: &Url) -> Option<Url> {
    let host = url.host_str()?;
    if !host_matches(host, "vimeo.com") || is_vimeo_player(url) {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    if segments
        .first()
        .is_some_and(|first| VIMEO_COLLECTIONS.contains(first))
    {
        return None;
    }
    let position = segments.iter().position(|segment| {
        segment.len() >= MIN_VIMEO_ID_DIGITS && segment.bytes().all(|b| b.is_ascii_digit())
    })?;
    let id = segments[position];
    let hash = segments
        .get(position + 1)
        .filter(|hash| {
            hash.len() <= MAX_VIMEO_HASH_CHARS && hash.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .map(|hash| hash.to_string())
        .or_else(|| {
            url.query_pairs()
                .find(|(key, _)| key == "h")
                .map(|(_, v)| v.into_owned())
        })
        .filter(|hash| !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit()));
    let mut player = Url::parse("https://player.vimeo.com/video/")
        .ok()?
        .join(id)
        .ok()?;
    if let Some(hash) = hash {
        player.query_pairs_mut().append_pair("h", &hash);
    }
    Some(player)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(input: &str) -> (String, Vec<String>) {
        let (url, args) = public_route(&Url::parse(input).unwrap());
        (url.to_string(), args)
    }

    #[test]
    fn vimeo_pages_use_the_public_player_with_a_vimeo_referer() {
        for (input, expected) in [
            (
                "https://vimeo.com/76979871",
                "https://player.vimeo.com/video/76979871",
            ),
            (
                "https://www.vimeo.com/76979871?share=copy",
                "https://player.vimeo.com/video/76979871",
            ),
            (
                "https://vimeo.com/76979871/1a2b3c4d5e",
                "https://player.vimeo.com/video/76979871?h=1a2b3c4d5e",
            ),
            (
                "https://vimeo.com/channels/staffpicks/76979871",
                "https://player.vimeo.com/video/76979871",
            ),
            (
                "https://vimeo.com/groups/shortfilms/videos/76979871",
                "https://player.vimeo.com/video/76979871",
            ),
            (
                "https://player.vimeo.com/video/76979871?h=abc123",
                "https://player.vimeo.com/video/76979871?h=abc123",
            ),
        ] {
            let (url, args) = route(input);
            assert_eq!(url, expected, "{input}");
            assert_eq!(args, ["--referer", "https://vimeo.com/"]);
        }
    }

    #[test]
    fn leaves_collections_lookalikes_and_other_sites_alone() {
        for input in [
            "https://vimeo.com/showcase/1234567",
            "https://vimeo.com/user1234567",
            "https://vimeo.com.evil.example/76979871",
            "https://www.youtube.com/watch?v=jNQXAC9IVRw",
        ] {
            let (url, args) = route(input);
            assert_eq!(url, Url::parse(input).unwrap().to_string(), "{input}");
            assert!(args.is_empty(), "{input}");
        }
    }
}
