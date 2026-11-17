//! A partner for the client tests that plays the broker: it speaks envelopes on a
//! stream socket or a WebSocket and is driven step by step by the test.

#![allow(dead_code)] // Each test file uses part of the harness.

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tilr_bus::bus::{Envelope, HelloAck, envelope::Msg};
use tilr_bus::{Transport, codec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::{Message as WsMessage, http::HeaderValue};

pub const WAIT: Duration = Duration::from_secs(10);

pub fn env(msg: Msg) -> Envelope {
    Envelope { msg: Some(msg) }
}

pub fn ack(identity: &str) -> HelloAck {
    HelloAck {
        protocol_version: 1,
        identity: identity.into(),
        connection_id: 7,
        max_frame: tilr_bus::limits::MAX_FRAME,
        max_in_flight: tilr_bus::limits::MAX_IN_FLIGHT,
        stream_window: tilr_bus::limits::STREAM_WINDOW,
        connection_window: tilr_bus::limits::CONNECTION_WINDOW,
        credential: Bytes::new(),
        server_id: "server-1".into(),
    }
}

enum Link {
    Stream(UnixStream, BytesMut),
    Ws(Box<tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>>),
}

/// Records the request headers and agrees on the subprotocol.
struct Handshake<'a>(&'a mut Vec<(String, String)>);

impl Callback for Handshake<'_> {
    fn on_request(
        self,
        request: &Request,
        mut response: Response,
    ) -> Result<Response, ErrorResponse> {
        for (name, value) in request.headers() {
            self.0
                .push((name.to_string(), value.to_str().unwrap_or("").to_owned()));
        }
        response.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_static(tilr_bus::SUBPROTOCOL),
        );
        Ok(response)
    }
}

/// The broker's end of one connection.
pub struct Peer {
    link: Link,
    /// The request headers of a WebSocket handshake.
    pub headers: Vec<(String, String)>,
}

impl Peer {
    /// A socket pair: the client's end as `Transport::Fd`.
    pub fn pair() -> (Transport, Peer) {
        let (client, broker) = std::os::unix::net::UnixStream::pair().unwrap();
        broker.set_nonblocking(true).unwrap();
        let broker = UnixStream::from_std(broker).unwrap();
        (
            Transport::Fd(client.into()),
            Peer {
                link: Link::Stream(broker, BytesMut::new()),
                headers: Vec::new(),
            },
        )
    }

    pub fn from_unix(stream: UnixStream) -> Peer {
        Peer {
            link: Link::Stream(stream, BytesMut::new()),
            headers: Vec::new(),
        }
    }

    /// Accept one WebSocket connection on `listener`, agreeing on `tilr.bus.v1`.
    pub async fn accept_ws(listener: &TcpListener) -> Peer {
        let (socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        let ws = tokio_tungstenite::accept_hdr_async(socket, Handshake(&mut headers))
            .await
            .unwrap();
        Peer {
            link: Link::Ws(Box::new(ws)),
            headers,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub async fn send(&mut self, msg: Msg) {
        let envelope = env(msg);
        match &mut self.link {
            Link::Stream(stream, _) => {
                stream
                    .write_all(&codec::encode_stream(&envelope).unwrap())
                    .await
                    .unwrap();
            }
            Link::Ws(ws) => {
                ws.send(WsMessage::Binary(codec::encode_ws(&envelope).unwrap()))
                    .await
                    .unwrap();
            }
        }
    }

    /// Several envelopes in one write on a stream socket: the client reads them
    /// together.
    pub async fn send_together(&mut self, msgs: Vec<Msg>) {
        let Link::Stream(stream, _) = &mut self.link else {
            panic!("not a stream socket");
        };
        let mut bytes = Vec::new();
        for msg in msgs {
            bytes.extend_from_slice(&codec::encode_stream(&env(msg)).unwrap());
        }
        stream.write_all(&bytes).await.unwrap();
    }

    pub async fn send_raw_ws(&mut self, message: WsMessage) {
        let Link::Ws(ws) = &mut self.link else {
            panic!("not a websocket");
        };
        ws.send(message).await.unwrap();
    }

    /// The next envelope, or `None` once the client closed the connection.
    pub async fn try_recv(&mut self) -> Option<Envelope> {
        tokio::time::timeout(WAIT, self.recv_inner())
            .await
            .expect("the client sent nothing in time")
    }

    async fn recv_inner(&mut self) -> Option<Envelope> {
        match &mut self.link {
            Link::Stream(stream, buf) => loop {
                if let Some(envelope) = codec::decode_stream(buf).unwrap() {
                    return Some(envelope);
                }
                if stream.read_buf(buf).await.unwrap() == 0 {
                    return None;
                }
            },
            Link::Ws(ws) => loop {
                match ws.next().await {
                    Some(Ok(WsMessage::Binary(message))) => {
                        return Some(codec::decode_ws(&message).unwrap());
                    }
                    Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => return None,
                    Some(Ok(_)) => {}
                }
            },
        }
    }

    pub async fn recv(&mut self) -> Msg {
        self.try_recv()
            .await
            .expect("connection closed")
            .msg
            .expect("an envelope of a known kind")
    }

    /// Whether nothing arrives for a while.
    pub async fn quiet(&mut self, wait: Duration) -> bool {
        tokio::time::timeout(wait, self.recv_inner()).await.is_err()
    }

    /// Take the client's Hello and answer `ack`.
    pub async fn greet(&mut self, ack: HelloAck) -> tilr_bus::bus::Hello {
        let Msg::Hello(hello) = self.recv().await else {
            panic!("the first envelope is not Hello");
        };
        self.send(Msg::HelloAck(ack)).await;
        hello
    }

    pub async fn close_ws(&mut self, code: u16, reason: &str) {
        let Link::Ws(ws) = &mut self.link else {
            panic!("not a websocket");
        };
        ws.close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
            code: code.into(),
            reason: reason.to_owned().into(),
        }))
        .await
        .unwrap();
        // Drain until the client's close comes back.
        while let Some(Ok(_)) = ws.next().await {}
    }
}
