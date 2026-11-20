//! A loopback CONNECT proxy that every yt-dlp run must use.
//!
//! yt-dlp resolves names, follows redirects, embedded players and stream
//! segments by itself. Routing it through this proxy means every outbound
//! connection is resolved and checked here, then pinned to the checked
//! address, with the same public-HTTPS rules as Pawket's own downloads.

use crate::{network, PawketError, Result};
use std::{io, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};
use url::Url;

const MAX_HEAD_BYTES: usize = 8 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Pawket's own downloads only use HTTPS on the default port.
const ALLOWED_PORT: u16 = 443;
const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";
const FORBIDDEN: &[u8] =
    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Runs until dropped. Open tunnels end when yt-dlp closes them.
pub(crate) struct GuardProxy {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl GuardProxy {
    pub(crate) async fn start() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                tokio::spawn(async move {
                    // A refused or broken tunnel only affects that request;
                    // yt-dlp reports the failure itself.
                    let _ = handle(client).await;
                });
            }
        });
        Ok(Self { address, task })
    }

    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for GuardProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle(mut client: TcpStream) -> io::Result<()> {
    let Ok(Ok((head, early))) = timeout(HEAD_TIMEOUT, read_head(&mut client)).await else {
        return Ok(());
    };
    let Some(target) = connect_target(&head) else {
        return client.write_all(FORBIDDEN).await;
    };
    let Ok(mut upstream) = connect_public(&target).await else {
        return client.write_all(FORBIDDEN).await;
    };
    client.write_all(ESTABLISHED).await?;
    if !early.is_empty() {
        upstream.write_all(&early).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// The request head, plus any bytes the client sent after it.
async fn read_head(client: &mut TcpStream) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = client.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        data.extend_from_slice(&chunk[..read]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let early = data.split_off(end + 4);
            return Ok((data, early));
        }
        if data.len() > MAX_HEAD_BYTES {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
}

/// `CONNECT host:443 HTTP/1.1` becomes `https://host/` after Pawket's URL
/// rules. Plain-HTTP proxy requests and other ports are refused.
fn connect_target(head: &[u8]) -> Option<Url> {
    let line = std::str::from_utf8(head).ok()?.lines().next()?;
    let mut parts = line.split(' ');
    let (method, authority, version) = (parts.next()?, parts.next()?, parts.next()?);
    if method != "CONNECT" || !version.starts_with("HTTP/1.") || parts.next().is_some() {
        return None;
    }
    let (host, port) = authority.rsplit_once(':')?;
    let is_plain_host = !host.is_empty() && !host.contains(['/', '@', '?', '#', '\\']);
    if port.parse::<u16>().ok()? != ALLOWED_PORT || !is_plain_host {
        return None;
    }
    network::validate_url(&format!("https://{host}/")).ok()
}

async fn connect_public(target: &Url) -> Result<TcpStream> {
    for address in network::public_addresses(target).await? {
        if let Ok(Ok(stream)) = timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await {
            return Ok(stream);
        }
    }
    Err(PawketError::Network("Couldn’t connect".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_public_https_connect_requests() {
        let target = |line: &str| connect_target(format!("{line}\r\n\r\n").as_bytes());
        assert_eq!(
            target("CONNECT www.youtube.com:443 HTTP/1.1").map(|u| u.to_string()),
            Some("https://www.youtube.com/".into())
        );
        assert!(target("CONNECT [2606:4700:4700::1111]:443 HTTP/1.1").is_some());
        for refused in [
            "GET http://example.org/video.mp4 HTTP/1.1",
            "CONNECT example.org:80 HTTP/1.1",
            "CONNECT 127.0.0.1:443 HTTP/1.1",
            "CONNECT 169.254.169.254:443 HTTP/1.1",
            "CONNECT localhost:443 HTTP/1.1",
            "CONNECT [::1]:443 HTTP/1.1",
            "CONNECT user@example.org:443 HTTP/1.1",
            "CONNECT example.org/x:443 HTTP/1.1",
            "CONNECT example.org:443 HTTP/1.1 extra",
            "CONNECT :443 HTTP/1.1",
        ] {
            assert_eq!(target(refused), None, "accepted {refused}");
        }
    }

    async fn ask(proxy: &GuardProxy, request: &str) -> String {
        let mut stream = TcpStream::connect(proxy.address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        reply
    }

    #[tokio::test]
    async fn refuses_private_targets_and_plain_requests_over_the_wire() {
        let proxy = GuardProxy::start().await.unwrap();
        for request in [
            "CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n",
            "CONNECT localtest.me:443 HTTP/1.1\r\n\r\n",
            "GET http://example.org/ HTTP/1.1\r\nHost: example.org\r\n\r\n",
        ] {
            assert!(
                ask(&proxy, request).await.starts_with("HTTP/1.1 403"),
                "{request}"
            );
        }
    }
}
