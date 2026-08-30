//! Outbound proxy support for SSH / SFTP connections (issue #7).
//!
//! Establishes the TCP stream to the target host **through a proxy**, then the
//! caller hands that stream to `russh::client::connect_stream`.  Both proxy
//! kinds end up as a transparent `TcpStream`:
//!
//! * **SOCKS5** (`socks5://` / `socks5h://`) via `tokio-socks`; after the
//!   handshake we unwrap to the inner `TcpStream`.
//! * **HTTP CONNECT** (`http://`): we issue an HTTP `CONNECT host:port` and reuse
//!   the same socket as the tunnel. `https://` proxies are *rejected* (see
//!   [`parse`]) because there is no TLS-proxy support — silently downgrading
//!   them to plaintext would leak the `Proxy-Authorization` credentials.
//!
//! The proxy is taken from the per-session setting, falling back to the standard
//! `ALL_PROXY` / `all_proxy` environment variable.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use zeroize::Zeroizing;

use crate::config::Secret;
use super::structs::{ProxyConfig, ProxyKind};

/// Resolve the proxy for a session: the explicit `session_proxy` string if set,
/// otherwise the `ALL_PROXY` / `all_proxy` environment variable.  Returns `None`
/// for a direct connection.
pub fn resolve(session_proxy: &str) -> Option<ProxyConfig> {
    let s = session_proxy.trim();
    if !s.is_empty() {
        return match parse(s) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("proxy config rejected: {e:#}");
                None
            }
        };
    }
    for var in ["ALL_PROXY", "all_proxy"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return match parse(v.trim()) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("proxy config rejected: {e:#}");
                        None
                    }
                };
            }
        }
    }
    None
}

/// Parse a proxy URL: `scheme://[user:pass@]host:port`.
fn parse(url: &str) -> Result<Option<ProxyConfig>> {
    let (scheme, rest) = url.split_once("://").unwrap_or(("socks5", url));
    let kind = match scheme.to_ascii_lowercase().as_str() {
        "socks5" | "socks5h" | "socks" => ProxyKind::Socks5,
        "http" => ProxyKind::Http,
        // No TLS-proxy support here: refusing the scheme is safer than silently
        // downgrading to a plaintext CONNECT tunnel, which would send the proxy
        // credentials (`Proxy-Authorization: Basic`) in clear text (#25).
        "https" => bail!(
            "HTTPS proxy ({scheme}://) is not supported; refusing to tunnel CONNECT \
             in plaintext — use an HTTP or SOCKS5 proxy instead"
        ),
        _ => return Ok(None),
    };
    // Optional userinfo before '@'.
    let (auth, hostport) = match rest.rsplit_once('@') {
        Some((userinfo, hp)) => {
            let (u, p) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            (Some((u.to_string(), Secret::new(p))), hp)
        }
        None => (None, rest),
    };
    let hostport = hostport.trim_end_matches('/');
    let (host, port) = match hostport.rsplit_once(':') {
        Some(v) => v,
        None => return Ok(None), // not `host:port` → no proxy
    };
    let port: u16 = match port.parse() {
        Ok(v) => v,
        Err(_) => return Ok(None), // non-numeric port → no proxy
    };
    if host.is_empty() {
        return Ok(None);
    }
    Ok(Some(ProxyConfig {
        kind,
        host: host.to_string(),
        port,
        auth,
    }))
}

/// Human-readable description of where we're connecting (for status messages).
pub fn describe(cfg: &ProxyConfig) -> String {
    let scheme = match cfg.kind {
        ProxyKind::Socks5 => "socks5",
        ProxyKind::Http => "http",
    };
    format!("{}://{}:{}", scheme, cfg.host, cfg.port)
}

/// Open a TCP stream to `target_host:target_port` through the proxy.
pub async fn connect(cfg: &ProxyConfig, target_host: &str, target_port: u16) -> Result<TcpStream> {
    match cfg.kind {
        ProxyKind::Socks5 => connect_socks5(cfg, target_host, target_port).await,
        ProxyKind::Http => connect_http(cfg, target_host, target_port).await,
    }
}

async fn connect_socks5(cfg: &ProxyConfig, host: &str, port: u16) -> Result<TcpStream> {
    use tokio_socks::tcp::Socks5Stream;
    let proxy = (cfg.host.as_str(), cfg.port);
    let target = (host, port);
    let stream = match &cfg.auth {
        Some((u, p)) => Socks5Stream::connect_with_password(proxy, target, u, p.as_str())
            .await
            .context("SOCKS5 proxy connect failed")?,
        None => Socks5Stream::connect(proxy, target)
            .await
            .context("SOCKS5 proxy connect failed")?,
    };
    // After the handshake the underlying socket is a transparent tunnel.
    Ok(stream.into_inner())
}

async fn connect_http(cfg: &ProxyConfig, host: &str, port: u16) -> Result<TcpStream> {
    let mut s = TcpStream::connect((cfg.host.as_str(), cfg.port))
        .await
        .with_context(|| format!("connect to HTTP proxy {}:{} failed", cfg.host, cfg.port))?;

    // The request carries `Proxy-Authorization: Basic base64(user:pass)`; hold
    // both the token and the assembled request in zeroized buffers so the proxy
    // credentials don't linger in freed heap after the send (#26).
    let mut req = Zeroizing::new(format!(
        "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n"
    ));
    if let Some((u, p)) = &cfg.auth {
        let creds = Zeroizing::new(format!("{u}:{}", p.as_str()));
        let token = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(&*creds));
        req.push_str("Proxy-Authorization: Basic ");
        req.push_str(&*token);
        req.push_str("\r\n");
    }
    req.push_str("Proxy-Connection: keep-alive\r\n\r\n");
    s.write_all(req.as_bytes())
        .await
        .context("write CONNECT to proxy")?;

    // Read response headers up to the blank line, bounded.
    let mut buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = s.read(&mut byte).await.context("read proxy response")?;
        if n == 0 {
            bail!("proxy closed the connection during CONNECT");
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 8192 {
            bail!("proxy CONNECT response too large");
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let status_line = head.lines().next().unwrap_or("");
    // Expect "HTTP/1.x 200 ...".
    let ok = status_line
        .split_whitespace()
        .nth(1)
        .map(|c| c == "200")
        .unwrap_or(false);
    if !ok {
        return Err(anyhow!("proxy CONNECT rejected: {}", status_line.trim()));
    }
    Ok(s)
}
