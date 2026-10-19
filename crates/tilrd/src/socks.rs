//! The SOCKS proxy: lets a client open TCP connections from the server's
//! machine, so that a browser on another one reaches the services there that
//! listen on 127.0.0.1 only. SOCKS5 (RFC 1928), CONNECT only, with names
//! resolved on the server ("remote DNS"): `localhost` is the server's.
//!
//! Two ways in, both on the server's own port and both off unless the server
//! runs with `--socks` (or `socks = true` in its configuration file):
//!
//! - `GET /socks`, a WebSocket whose binary messages carry the SOCKS bytes
//!   ([`serve_websocket`]). The handshake has already shown the access token,
//!   so SOCKS asks for none. This is the way through a tilr Cloud tunnel.
//! - Raw SOCKS5: a connection whose first byte is 5 is a SOCKS client, which
//!   must give the access token as its password (RFC 1929). For `ssh -L` and
//!   the like; a tunnel only carries HTTP.

use std::io::ErrorKind;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(feature = "failpoints")]
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::auth;

/// The path of the WebSocket that carries SOCKS.
pub const PATH: &str = "/socks";
/// The first byte a SOCKS5 client sends, and how a raw session is told from
/// an HTTP request on the same port.
pub const VERSION: u8 = 5;

/// How long a client has to say where it wants to go, and the server to get
/// there.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Bytes in flight between a WebSocket and its SOCKS session, each way.
const PIPE_BYTES: usize = 64 * 1024;
/// Messages queued for a WebSocket before the session waits for it.
const WS_QUEUE: usize = 16;
/// Pings sent within the time a WebSocket peer may stay silent.
const PINGS_PER_TIMEOUT: u32 = 3;

const NO_AUTH: u8 = 0x00;
const PASSWORD: u8 = 0x02;
const NO_METHOD: u8 = 0xff;
const PASSWORD_VERSION: u8 = 0x01;

const CONNECT: u8 = 0x01;

const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

const SUCCEEDED: u8 = 0x00;
const FAILURE: u8 = 0x01;
const NETWORK_UNREACHABLE: u8 = 0x03;
const HOST_UNREACHABLE: u8 = 0x04;
const REFUSED: u8 = 0x05;
const COMMAND_NOT_SUPPORTED: u8 = 0x07;
const ADDRESS_NOT_SUPPORTED: u8 = 0x08;

/// What a SOCKS client must show before it may connect.
pub enum Auth {
    /// Nothing: the transport has let it in already. The client may offer
    /// no authentication or a username and password; any are taken.
    None,
    /// A username and password (RFC 1929), the password being the access
    /// token. Any username.
    Password { token: String },
}

/// One SOCKS session on `stream`: the negotiation, the connection to the
/// target, then bytes both ways until both sides are done. `peer` is who
/// is on the other end, for the log.
pub async fn serve<S>(mut stream: S, auth: Auth, peer: SocketAddr) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_session(&mut stream, auth, peer, true).await
}

async fn serve_session<S>(
    mut stream: S,
    auth: Auth,
    peer: SocketAddr,
    test_clock: bool,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let started = Instant::now();
    #[cfg(not(feature = "failpoints"))]
    let _ = test_clock;
    #[cfg(feature = "failpoints")]
    let negotiated = if test_clock && let Some(path) = crate::time::test_time_file() {
        let start_us = crate::time::test_time_us().expect("test clock path is set");
        let deadline_us = start_us.saturating_add(HANDSHAKE_TIMEOUT.as_micros() as u64);
        // The method selection sent by `negotiate` is the barrier proving
        // this listener is ready. The test updates the clock, then wakes it.
        let control = TcpListener::bind("127.0.0.1:0").await?;
        std::fs::write(
            path.with_extension(format!("socks-{}", peer.port())),
            control.local_addr()?.port().to_string(),
        )?;
        let expires = async {
            loop {
                let (mut signal, _) = control.accept().await?;
                if crate::time::test_time_us().expect("test clock path is set") >= deadline_us {
                    // Acknowledge only this deadline, so tests can distinguish
                    // it from the unchanged ten-second real-time limit.
                    signal.write_all(&[1]).await?;
                    break Ok::<(), anyhow::Error>(());
                }
            }
        };
        // Keep the production limit too: a stationary test clock must never
        // leave a raw session open beyond ten real seconds.
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            tokio::select! {
                result = negotiate(&mut stream, &auth, peer) => result,
                result = expires => {
                    result?;
                    bail!("SOCKS client {peer} said nothing in time");
                }
            }
        })
        .await
        {
            Ok(result) => result?,
            Err(_) => bail!("SOCKS client {peer} said nothing in time"),
        }
    } else {
        timed_negotiate(&mut stream, &auth, peer).await?
    };
    #[cfg(not(feature = "failpoints"))]
    let negotiated = timed_negotiate(&mut stream, &auth, peer).await?;
    let Some((mut target, name)) = negotiated else {
        return Ok(());
    };
    let _ = target.set_nodelay(true);
    let copied = tokio::io::copy_bidirectional(&mut stream, &mut target).await;
    let elapsed = started.elapsed();
    match copied {
        Ok((up, down)) => {
            tracing::debug!(%peer, target = %name, up, down, ?elapsed, "SOCKS session ended")
        }
        Err(err) => tracing::debug!(%peer, target = %name, ?elapsed, "SOCKS session broke: {err}"),
    }
    Ok(())
}

async fn timed_negotiate<S>(
    stream: &mut S,
    auth: &Auth,
    peer: SocketAddr,
) -> Result<Option<(TcpStream, String)>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, negotiate(stream, auth, peer)).await {
        Ok(result) => result,
        Err(_) => bail!("SOCKS client {peer} said nothing in time"),
    }
}

/// The handshake up to the reply: the connection to the target and its name
/// for the log, or `None` if the client was turned away.
async fn negotiate<S>(
    stream: &mut S,
    auth: &Auth,
    peer: SocketAddr,
) -> Result<Option<(TcpStream, String)>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Greeting: VER, NMETHODS, METHODS.
    let [version, count] = read_array(stream).await?;
    if version != VERSION {
        bail!("not a SOCKS5 client (version {version})");
    }
    let mut methods = vec![0; usize::from(count)];
    stream.read_exact(&mut methods).await?;
    let method = match auth {
        Auth::None if methods.contains(&NO_AUTH) => NO_AUTH,
        Auth::None | Auth::Password { .. } if methods.contains(&PASSWORD) => PASSWORD,
        _ => NO_METHOD,
    };
    stream.write_all(&[VERSION, method]).await?;
    if method == NO_METHOD {
        tracing::warn!(%peer, ?methods, "SOCKS client offered no method it may use");
        return Ok(None);
    }
    if method == PASSWORD && !check_password(stream, auth, peer).await? {
        return Ok(None);
    }

    // Request: VER, CMD, RSV, ATYP, DST.ADDR, DST.PORT.
    let [version, command, _, kind] = read_array(stream).await?;
    if version != VERSION {
        bail!("SOCKS request of version {version}");
    }
    let host = match kind {
        ATYP_IPV4 => Host::Ip(Ipv4Addr::from(read_array::<4, _>(stream).await?).into()),
        ATYP_IPV6 => Host::Ip(Ipv6Addr::from(read_array::<16, _>(stream).await?).into()),
        ATYP_DOMAIN => {
            let [len] = read_array(stream).await?;
            let mut name = vec![0; usize::from(len)];
            stream.read_exact(&mut name).await?;
            Host::Name(String::from_utf8(name).context("SOCKS domain name is not UTF-8")?)
        }
        _ => {
            reply(stream, ADDRESS_NOT_SUPPORTED, None).await?;
            return Ok(None);
        }
    };
    let port = u16::from_be_bytes(read_array(stream).await?);
    let name = match &host {
        Host::Ip(ip) => SocketAddr::new(*ip, port).to_string(),
        Host::Name(name) => format!("{name}:{port}"),
    };
    if command != CONNECT {
        tracing::info!(%peer, command, target = %name, "SOCKS command not supported");
        reply(stream, COMMAND_NOT_SUPPORTED, None).await?;
        return Ok(None);
    }

    let target = match tokio::time::timeout(CONNECT_TIMEOUT, connect(&host, port)).await {
        Ok(Ok(target)) => target,
        Ok(Err(err)) => {
            tracing::warn!(%peer, target = %name, "SOCKS connect failed: {err}");
            reply(stream, reply_code(&err), None).await?;
            return Ok(None);
        }
        Err(_) => {
            tracing::warn!(%peer, target = %name, "SOCKS connect timed out");
            reply(stream, HOST_UNREACHABLE, None).await?;
            return Ok(None);
        }
    };
    reply(stream, SUCCEEDED, target.local_addr().ok()).await?;
    tracing::debug!(%peer, target = %name, "SOCKS connected");
    Ok(Some((target, name)))
}

/// RFC 1929: VER, ULEN, UNAME, PLEN, PASSWD; answered with VER, STATUS.
/// Whether the client may go on.
async fn check_password<S>(stream: &mut S, auth: &Auth, peer: SocketAddr) -> Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let [version, len] = read_array(stream).await?;
    if version != PASSWORD_VERSION {
        bail!("SOCKS password negotiation of version {version}");
    }
    let mut username = vec![0; usize::from(len)];
    stream.read_exact(&mut username).await?;
    let [len] = read_array(stream).await?;
    let mut password = vec![0; usize::from(len)];
    stream.read_exact(&mut password).await?;
    let ok = match auth {
        Auth::None => true,
        Auth::Password { token } => {
            auth::matches(token, std::str::from_utf8(&password).unwrap_or(""))
        }
    };
    stream
        .write_all(&[PASSWORD_VERSION, if ok { 0x00 } else { 0x01 }])
        .await?;
    if !ok {
        tracing::warn!(%peer, "SOCKS client with a wrong access token");
    }
    Ok(ok)
}

enum Host {
    Ip(std::net::IpAddr),
    Name(String),
}

/// Connects to the host, a name being resolved here; tries each of its
/// addresses in turn and reports the last failure.
async fn connect(host: &Host, port: u16) -> std::io::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = match host {
        Host::Ip(ip) => vec![SocketAddr::new(*ip, port)],
        Host::Name(name) if is_localhost(name) => vec![
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
        ],
        Host::Name(name) => tokio::net::lookup_host((name.as_str(), port))
            .await
            .map_err(|err| std::io::Error::new(ErrorKind::HostUnreachable, err))?
            .collect(),
    };
    let mut last = std::io::Error::new(ErrorKind::HostUnreachable, "the name has no address");
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// Whether a name is this machine's loopback by RFC 6761: `localhost` and
/// every name under it, never asked of DNS. A browser connects to
/// `localhost` itself rather than through a proxy, so a page reaches the
/// server's loopback by another name, e.g. `app.localhost:3000`.
fn is_localhost(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase();
    name == "localhost" || name.ends_with(".localhost")
}

/// The SOCKS reply for a connection that failed.
fn reply_code(err: &std::io::Error) -> u8 {
    match err.kind() {
        ErrorKind::ConnectionRefused => REFUSED,
        ErrorKind::NetworkUnreachable => NETWORK_UNREACHABLE,
        ErrorKind::HostUnreachable | ErrorKind::TimedOut => HOST_UNREACHABLE,
        _ => FAILURE,
    }
}

/// VER, REP, RSV, then the address the server connected from (BND.ADDR and
/// BND.PORT); 0.0.0.0:0 when there is none.
async fn reply<S>(stream: &mut S, code: u8, bound: Option<SocketAddr>) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let mut message = vec![VERSION, code, 0];
    match bound.unwrap_or_else(|| SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))) {
        SocketAddr::V4(addr) => {
            message.push(ATYP_IPV4);
            message.extend(addr.ip().octets());
            message.extend(addr.port().to_be_bytes());
        }
        SocketAddr::V6(addr) => {
            message.push(ATYP_IPV6);
            message.extend(addr.ip().octets());
            message.extend(addr.port().to_be_bytes());
        }
    }
    stream.write_all(&message).await?;
    Ok(())
}

async fn read_array<const N: usize, S>(stream: &mut S) -> std::io::Result<[u8; N]>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = [0; N];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

/// A SOCKS session over a WebSocket: the client's binary messages are the
/// bytes it sends, the server's are the bytes it gets back, in any sizes.
///
/// A text message breaks the rules and closes the WebSocket with 1003. A
/// WebSocket cannot be half closed: when the target is done sending, the
/// server closes it (1000) and the session ends once the client answers;
/// when the client closes it, the session ends. Pings keep it alive the way
/// they keep the main protocol's: a peer silent for `timeout` is gone.
pub async fn serve_websocket<S>(ws: WebSocketStream<S>, timeout: Option<Duration>, peer: SocketAddr)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (pipe, session_end) = tokio::io::duplex(PIPE_BYTES);
    let (mut from_session, mut to_session) = tokio::io::split(pipe);
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Message>(WS_QUEUE);

    // Sends what the rest queue for the peer; done once they are, or once
    // the WebSocket is closed.
    let writer = async move {
        while let Some(message) = rx.recv().await {
            let close = matches!(message, Message::Close(_));
            if sink.send(message).await.is_err() || close {
                break;
            }
        }
        let _ = sink.close().await;
    };

    // The client's bytes, to the session; done when the client is.
    let upstream = {
        let tx = tx.clone();
        async move {
            loop {
                let message = match timeout {
                    Some(timeout) => match tokio::time::timeout(timeout, source.next()).await {
                        Ok(message) => message,
                        Err(_) => {
                            tracing::warn!(%peer, "SOCKS WebSocket went silent, dropping");
                            break;
                        }
                    },
                    None => source.next().await,
                };
                match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if to_session.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(_))) => {
                        tracing::warn!(%peer, "SOCKS WebSocket sent text, closing");
                        let _ = tx
                            .send(Message::Close(Some(CloseFrame {
                                code: CloseCode::Unsupported,
                                reason: "binary messages only".into(),
                            })))
                            .await;
                        break;
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
            let _ = to_session.shutdown().await;
        }
    };

    // The session's bytes, to the client; then the close.
    let downstream = {
        let tx = tx.clone();
        async move {
            let mut ping = timeout.map(|timeout| {
                let mut ping = tokio::time::interval(timeout / PINGS_PER_TIMEOUT);
                ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
                ping
            });
            let mut buf = vec![0; PIPE_BYTES];
            loop {
                let message = tokio::select! {
                    read = from_session.read(&mut buf) => match read {
                        Ok(0) | Err(_) => break,
                        Ok(n) => Message::Binary(Bytes::copy_from_slice(&buf[..n])),
                    },
                    () = tick(&mut ping) => Message::Ping(Bytes::new()),
                };
                if tx.send(message).await.is_err() {
                    return;
                }
            }
            let _ = tx
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "".into(),
                })))
                .await;
        }
    };

    let session = async move {
        if let Err(err) = serve_session(session_end, Auth::None, peer, false).await {
            tracing::debug!(%peer, "SOCKS session over WebSocket: {err:#}");
        }
    };

    // Everything but the writer holds a sender; when they are done, so is
    // the writer.
    let feed = async move {
        tokio::select! {
            () = upstream => {}
            ((), ()) = async { tokio::join!(session, downstream) } => {}
        }
        drop(tx);
    };
    tokio::join!(feed, writer);
}

/// The next tick of `interval`; never, without one.
async fn tick(interval: &mut Option<tokio::time::Interval>) {
    match interval {
        Some(interval) => {
            interval.tick().await;
        }
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::is_localhost;

    #[test]
    fn localhost_and_the_names_under_it_are_loopback() {
        for name in [
            "localhost",
            "LocalHost",
            "pb.localhost",
            "a.b.LOCALHOST",
            "localhost.",
        ] {
            assert!(is_localhost(name), "{name}");
        }
        for name in ["localhost.com", "mylocalhost", "example.org", ""] {
            assert!(!is_localhost(name), "{name}");
        }
    }
}
