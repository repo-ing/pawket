use crate::{PawketError, Result};
use futures_util::StreamExt;
use reqwest::header::LOCATION;
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use url::{Host, Url};

const MAX_REDIRECTS: usize = 5;

pub fn validate_url(input: &str) -> Result<Url> {
    if input.len() > 8192 || input.chars().any(|c| c.is_control()) {
        return Err(PawketError::InvalidUrl);
    }
    let mut url = Url::parse(input.trim()).map_err(|_| PawketError::InvalidUrl)?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(PawketError::InvalidUrl);
    }
    if url.port().is_some_and(|p| p != 443) {
        return Err(PawketError::BlockedAddress);
    }
    match url.host() {
        Some(Host::Ipv4(ip)) if public_v4(ip) => {}
        Some(Host::Ipv6(ip)) if public_v6(ip) => {}
        Some(Host::Domain(host)) if public_hostname(host) => {}
        _ => return Err(PawketError::BlockedAddress),
    }
    url.set_fragment(None);
    Ok(url)
}

fn public_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if !host.contains('.') {
        return false;
    }
    ![
        "localhost",
        "local",
        "internal",
        "invalid",
        "test",
        "example",
        "onion",
        "home.arpa",
    ]
    .iter()
    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    // Reject non-global ranges including shared space, documentation,
    // benchmarking, multicast and address space reserved for future use.
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    // Only global unicast 2000::/3; reject documentation and transition
    // mechanisms which can embed otherwise-blocked IPv4 destinations.
    (s[0] & 0xe000) == 0x2000
        && s[0] != 0x2002
        && !(s[0] == 0x2001 && s[1] < 0x0200)
        && !(s[0] == 0x2001 && s[1] == 0x0db8)
        && !(s[0] == 0x3fff && (s[1] & 0xf000) == 0)
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

async fn addresses(url: &Url) -> Result<Vec<SocketAddr>> {
    let host = url.host_str().ok_or(PawketError::InvalidUrl)?;
    let addresses: Vec<SocketAddr> = match url.host() {
        Some(Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), 443)],
        Some(Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), 443)],
        _ => tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::lookup_host((host, 443)),
        )
        .await
        .map_err(|_| PawketError::Network("DNS lookup timed out".into()))?
        .map_err(|_| PawketError::Network("DNS lookup failed".into()))?
        .collect(),
    };
    if addresses.is_empty() {
        return Err(PawketError::Network("No address found".into()));
    }
    if addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(PawketError::BlockedAddress);
    }
    Ok(addresses)
}

/// The host's addresses, failing unless every one of them is public.
pub(crate) async fn public_addresses(url: &Url) -> Result<Vec<SocketAddr>> {
    addresses(url).await
}

pub async fn fetch(url: Url) -> Result<(Url, reqwest::Response)> {
    fetch_scoped(url, |_| true).await
}

/// Apply a provider's provenance rules before every request, including redirects.
pub async fn fetch_scoped(
    mut url: Url,
    allowed: fn(&Url) -> bool,
) -> Result<(Url, reqwest::Response)> {
    // Never silently bypass an operator-configured outbound proxy. The
    // direct transport below is safe only when direct egress is permitted.
    if [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .iter()
    .any(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()))
    {
        return Err(PawketError::ProxyUnsupported);
    }
    for redirects in 0..=MAX_REDIRECTS {
        url = validate_url(url.as_str())?;
        if !allowed(&url) {
            return Err(PawketError::BlockedAddress);
        }
        let addresses = addresses(&url).await?;
        let host = url.host_str().ok_or(PawketError::InvalidUrl)?;
        // A fresh resolver override pins the checked destination for each hop.
        // Proxy use is intentionally unsupported: a proxy can resolve a
        // hostname independently, invalidating this application's DNS pin.
        // Deploy behind an ingress proxy; outbound requests are direct HTTPS.
        let client = reqwest::Client::builder()
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(180))
            .user_agent(concat!(
                "Pawket/",
                env!("CARGO_PKG_VERSION"),
                " (Rust media client)"
            ))
            .build()?;
        let response = client.get(url.clone()).send().await?;
        if response.status().is_redirection() {
            if redirects == MAX_REDIRECTS {
                return Err(PawketError::RedirectLimit);
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(PawketError::InvalidUrl)?;
            url = validate_url(
                url.join(location)
                    .map_err(|_| PawketError::InvalidUrl)?
                    .as_str(),
            )?;
            continue;
        }
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(PawketError::RateLimited);
        }
        if !response.status().is_success() {
            return Err(PawketError::Http(response.status().as_u16()));
        }
        return Ok((url, response));
    }
    Err(PawketError::RedirectLimit)
}

/// Read a whole response body, failing once it exceeds `limit` bytes.
pub async fn read_limited(response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > limit) {
        return Err(PawketError::TooLarge { limit });
    }
    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(next) = stream.next().await {
        let chunk = next?;
        if (data.len() as u64).saturating_add(chunk.len() as u64) > limit {
            return Err(PawketError::TooLarge { limit });
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_untrusted_schemes_credentials_and_ports() {
        for input in [
            "file:///etc/passwd",
            "http://example.com/file.mp4",
            "ftp://example.com/a",
            "https://u:p@example.com/a",
            "https://example.com:8080/a",
            "javascript:alert(1)",
            "https://example.com/a\nb",
        ] {
            assert!(validate_url(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn blocks_private_literal_encodings_and_special_names() {
        for input in [
            "https://127.0.0.1/a",
            "https://2130706433/a",
            "https://0x7f000001/a",
            "https://10.0.0.1/a",
            "https://169.254.169.254/a",
            "https://100.64.0.1/a",
            "https://192.168.1.1/a",
            "https://[::1]/a",
            "https://[::ffff:127.0.0.1]/a",
            "https://[fc00::1]/a",
            "https://[fe80::1]/a",
            "https://localhost./a",
            "https://service.internal/a",
            "https://printer.local/a",
            "https://intranet/a",
        ] {
            assert!(validate_url(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn allows_public_https_and_normalizes_fragment() {
        let url = validate_url(" https://example.com:443/clip.mp4?token=x#fragment ").unwrap();
        assert_eq!(url.as_str(), "https://example.com/clip.mp4?token=x");
        assert!(validate_url("https://8.8.8.8/media.mp4").is_ok());
        assert!(validate_url("https://[2606:4700:4700::1111]/media.mp4").is_ok());
    }

    #[test]
    fn rejects_reserved_ipv4_and_ipv6_ranges() {
        for addr in [
            "0.0.0.0",
            "224.0.0.1",
            "240.0.0.1",
            "192.0.2.10",
            "198.18.0.1",
            "203.0.113.1",
            "2001:db8::1",
            "2002:7f00:1::",
            "64:ff9b::7f00:1",
            "3fff::1",
        ] {
            assert!(!public_ip(addr.parse().unwrap()), "accepted {addr}");
        }
    }
}
