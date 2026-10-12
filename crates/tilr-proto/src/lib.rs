//! Wire protocol shared by the tilr server and its clients.
//!
//! Every message travels as one packet: a flag byte followed by the protobuf
//! body, zstd-compressed when the flag says so. On WebSocket a packet is one
//! binary message; on the internal pane socket it is length-prefixed.

use bytes::{BufMut, Bytes, BytesMut};
use prost::Message;

include!(concat!(env!("OUT_DIR"), "/tilr.v1.rs"));

pub const PROTOCOL_VERSION: u32 = 2;

/// Modifier bits used in [`KeyInput::mods`] and [`MouseInput::mods`].
pub mod mods {
    pub const SHIFT: u32 = 1;
    pub const ALT: u32 = 2;
    pub const CTRL: u32 = 4;
    pub const SUPER: u32 = 8;
}

/// Attribute bits used in [`Style::attrs`].
pub mod attrs {
    pub const BOLD: u32 = 1;
    pub const ITALIC: u32 = 2;
    pub const FAINT: u32 = 4;
    pub const BLINK: u32 = 8;
    pub const INVERSE: u32 = 16;
    pub const INVISIBLE: u32 = 32;
    pub const STRIKETHROUGH: u32 = 64;
    pub const OVERLINE: u32 = 128;
}

const FLAG_PLAIN: u8 = 0;
const FLAG_ZSTD: u8 = 1;

/// Bodies smaller than this are sent uncompressed: zstd would not pay for
/// itself on a keystroke or a one-row diff.
const COMPRESS_THRESHOLD: usize = 256;
const ZSTD_LEVEL: i32 = 1;
/// Upper bound for a decompressed body, so a hostile packet cannot blow up
/// memory.
const MAX_BODY: usize = 64 << 20;

#[derive(Debug)]
pub enum DecodeError {
    Empty,
    UnknownFlag(u8),
    Zstd(std::io::Error),
    Proto(prost::DecodeError),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty packet"),
            Self::UnknownFlag(flag) => write!(f, "unknown packet flag {flag}"),
            Self::Zstd(err) => write!(f, "zstd: {err}"),
            Self::Proto(err) => write!(f, "protobuf: {err}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Encode a message into a packet.
pub fn encode<M: Message>(msg: &M) -> Bytes {
    let len = msg.encoded_len();
    if len < COMPRESS_THRESHOLD {
        let mut buf = BytesMut::with_capacity(len + 1);
        buf.put_u8(FLAG_PLAIN);
        msg.encode(&mut buf).expect("buffer has capacity");
        return buf.freeze();
    }
    let body = msg.encode_to_vec();
    let mut out = Vec::with_capacity(body.len() / 2 + 16);
    out.push(FLAG_ZSTD);
    let mut encoder = zstd::stream::Encoder::new(out, ZSTD_LEVEL).expect("zstd encoder");
    std::io::Write::write_all(&mut encoder, &body).expect("write to vec");
    Bytes::from(encoder.finish().expect("write to vec"))
}

/// Decode a packet produced by [`encode`].
pub fn decode<M: Message + Default>(packet: &[u8]) -> Result<M, DecodeError> {
    let (&flag, body) = packet.split_first().ok_or(DecodeError::Empty)?;
    match flag {
        FLAG_PLAIN => M::decode(body).map_err(DecodeError::Proto),
        FLAG_ZSTD => {
            let decoder = zstd::stream::Decoder::new(body).map_err(DecodeError::Zstd)?;
            let mut plain = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::Read::take(decoder, MAX_BODY as u64),
                &mut plain,
            )
            .map_err(DecodeError::Zstd)?;
            M::decode(plain.as_slice()).map_err(DecodeError::Proto)
        }
        other => Err(DecodeError::UnknownFlag(other)),
    }
}

impl From<client_message::Msg> for ClientMessage {
    fn from(msg: client_message::Msg) -> Self {
        Self {
            msg: Some(msg),
            seq: 0,
        }
    }
}

impl ClientMessage {
    /// The message named by `seq`, see [`ClientMessage::seq`].
    pub fn with_seq(msg: client_message::Msg, seq: u64) -> Self {
        Self {
            msg: Some(msg),
            seq,
        }
    }
}

impl From<server_message::Msg> for ServerMessage {
    fn from(msg: server_message::Msg) -> Self {
        Self { msg: Some(msg) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rows: u32) -> ServerMessage {
        server_message::Msg::Frame(Frame {
            pane_id: "7".into(),
            seq: 1,
            full: true,
            cols: 80,
            rows,
            changed: (0..rows)
                .map(|y| Row {
                    y,
                    runs: vec![Run {
                        style: 0,
                        text: "hello world ".repeat(6),
                        wide: false,
                        cluster: false,
                    }],
                })
                .collect(),
            ..Default::default()
        })
        .into()
    }

    #[test]
    fn small_message_stays_plain() {
        let msg = frame(0);
        let packet = encode(&msg);
        assert_eq!(packet[0], FLAG_PLAIN);
        assert_eq!(decode::<ServerMessage>(&packet).unwrap(), msg);
    }

    #[test]
    fn large_message_is_compressed() {
        let msg = frame(40);
        let packet = encode(&msg);
        assert_eq!(packet[0], FLAG_ZSTD);
        assert!(packet.len() < msg.encoded_len());
        assert_eq!(decode::<ServerMessage>(&packet).unwrap(), msg);
    }

    #[test]
    fn input_seq_and_frame_timing_round_trip() {
        let input = ClientMessage::with_seq(
            client_message::Msg::Raw(RawInput {
                pane_id: "7".into(),
                data: Bytes::from_static(b"ls\n"),
            }),
            42,
        );
        let decoded = decode::<ClientMessage>(&encode(&input)).unwrap();
        assert_eq!(decoded.seq, 42);
        assert_eq!(decoded, input);

        let timing = FrameTiming {
            wait_us: 1,
            parse_us: 2,
            build_us: 3,
            encode_us: 4,
            output_bytes: 5,
            reads: 6,
            input_us: u32::MAX,
        };
        let mut msg = frame(40);
        if let Some(server_message::Msg::Frame(frame)) = &mut msg.msg {
            frame.last_input_seq = 42;
            frame.timing = Some(timing);
        }
        let decoded = decode::<ServerMessage>(&encode(&msg)).unwrap();
        let Some(server_message::Msg::Frame(frame)) = &decoded.msg else {
            panic!("not a frame: {decoded:?}");
        };
        assert_eq!(frame.last_input_seq, 42);
        assert_eq!(frame.timing, Some(timing));
        assert_eq!(decoded, msg);
    }

    #[test]
    fn a_message_without_seq_reads_as_zero() {
        let packet = encode(&ClientMessage::from(client_message::Msg::Ping(Ping {
            nonce: 1,
        })));
        assert_eq!(decode::<ClientMessage>(&packet).unwrap().seq, 0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            decode::<ServerMessage>(&[]),
            Err(DecodeError::Empty)
        ));
        assert!(matches!(
            decode::<ServerMessage>(&[9, 1, 2]),
            Err(DecodeError::UnknownFlag(9))
        ));
    }
}
