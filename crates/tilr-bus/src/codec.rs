//! Framing of [`Envelope`]s on the two kinds of transport (design B3).
//!
//! - WebSocket: one binary message is a flag byte ([`FLAG_PLAIN`] or [`FLAG_ZSTD`])
//!   followed by one `Envelope`. Envelopes of [`COMPRESS_THRESHOLD`] bytes or more
//!   are compressed with zstd level 1, unless they carry a payload that is
//!   compressed already (`Chunk.content_encoding` [`ENCODING_ZSTD`]): that one is
//!   sent plain, so a frame is compressed once, by its pane.
//! - Stream sockets (an instance's descriptor 3 or its instance socket): a u32
//!   big-endian length followed by one `Envelope`, never compressed.
//!
//! An envelope is at most [`MAX_FRAME`] bytes after decompression, both ways:
//! [`encode_ws`] and [`encode_stream`] refuse a longer one, the decoders stop at the
//! limit (a stream length prefix over it is refused before anything is buffered).

use bytes::{Buf, BufMut, Bytes, BytesMut};
use prost::Message;

use crate::bus::{Envelope, envelope};

/// Largest envelope, after decompression, in either direction (`HelloAck.max_frame`).
pub const MAX_FRAME: usize = 16 << 20;
/// WebSocket flag byte: the body is the plain protobuf.
pub const FLAG_PLAIN: u8 = 0;
/// WebSocket flag byte: the body is zstd-compressed.
pub const FLAG_ZSTD: u8 = 1;
/// Envelopes smaller than this go plain on a WebSocket: zstd would not pay for
/// itself on a keystroke or its reply.
pub const COMPRESS_THRESHOLD: usize = 256;
/// `Chunk.content_encoding` of a payload its producer compressed with zstd.
pub const ENCODING_ZSTD: &str = "zstd";
/// Bytes of the length prefix on a stream socket.
pub const LENGTH_PREFIX: usize = 4;

const ZSTD_LEVEL: i32 = 1;

/// Why an envelope could not be framed or read.
#[derive(Debug)]
pub enum CodecError {
    /// A WebSocket message with no bytes at all.
    Empty,
    /// A WebSocket flag byte other than 0 or 1.
    UnknownFlag(u8),
    /// The zstd body does not decompress.
    Zstd(std::io::Error),
    /// The body is not an `Envelope`.
    Proto(prost::DecodeError),
    /// The envelope is longer than [`MAX_FRAME`] (its length when known).
    TooLarge(usize),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty packet"),
            Self::UnknownFlag(flag) => write!(f, "unknown packet flag {flag}"),
            Self::Zstd(err) => write!(f, "zstd: {err}"),
            Self::Proto(err) => write!(f, "protobuf: {err}"),
            Self::TooLarge(len) => {
                write!(
                    f,
                    "envelope of {len} bytes is over the limit of {MAX_FRAME}"
                )
            }
        }
    }
}

impl std::error::Error for CodecError {}

/// Whether the envelope carries a payload its producer compressed already.
fn carries_compressed_payload(envelope: &Envelope) -> bool {
    matches!(&envelope.msg, Some(envelope::Msg::Chunk(chunk)) if chunk.content_encoding == ENCODING_ZSTD)
}

/// Frame an envelope as one binary WebSocket message.
pub fn encode_ws(envelope: &Envelope) -> Result<Bytes, CodecError> {
    let len = envelope.encoded_len();
    if len > MAX_FRAME {
        return Err(CodecError::TooLarge(len));
    }
    let plain = |len: usize| {
        let mut buf = BytesMut::with_capacity(len + 1);
        buf.put_u8(FLAG_PLAIN);
        envelope.encode(&mut buf).expect("buffer has capacity");
        buf.freeze()
    };
    if len < COMPRESS_THRESHOLD || carries_compressed_payload(envelope) {
        return Ok(plain(len));
    }
    let body = envelope.encode_to_vec();
    let mut out = Vec::with_capacity(body.len() / 2 + 16);
    out.push(FLAG_ZSTD);
    let mut encoder = zstd::stream::Encoder::new(out, ZSTD_LEVEL).expect("zstd encoder");
    std::io::Write::write_all(&mut encoder, &body).expect("write to vec");
    let compressed = encoder.finish().expect("write to vec");
    // Data that does not compress goes plain: never larger than the envelope
    // plus its flag byte, so an envelope at the limit fits any peer's limit.
    if compressed.len() > body.len() {
        return Ok(plain(len));
    }
    Ok(Bytes::from(compressed))
}

/// Read one binary WebSocket message produced by [`encode_ws`].
pub fn decode_ws(message: &[u8]) -> Result<Envelope, CodecError> {
    let (&flag, body) = message.split_first().ok_or(CodecError::Empty)?;
    match flag {
        FLAG_PLAIN => {
            if body.len() > MAX_FRAME {
                return Err(CodecError::TooLarge(body.len()));
            }
            Envelope::decode(body).map_err(CodecError::Proto)
        }
        FLAG_ZSTD => {
            let decoder = zstd::stream::Decoder::new(body).map_err(CodecError::Zstd)?;
            let mut plain = Vec::new();
            // One byte past the limit tells a body at the limit from a longer one.
            std::io::Read::read_to_end(
                &mut std::io::Read::take(decoder, MAX_FRAME as u64 + 1),
                &mut plain,
            )
            .map_err(CodecError::Zstd)?;
            if plain.len() > MAX_FRAME {
                return Err(CodecError::TooLarge(plain.len()));
            }
            Envelope::decode(plain.as_slice()).map_err(CodecError::Proto)
        }
        other => Err(CodecError::UnknownFlag(other)),
    }
}

/// Frame an envelope for a stream socket: u32 big-endian length, then the envelope.
pub fn encode_stream(envelope: &Envelope) -> Result<Bytes, CodecError> {
    let len = envelope.encoded_len();
    if len > MAX_FRAME {
        return Err(CodecError::TooLarge(len));
    }
    let mut buf = BytesMut::with_capacity(LENGTH_PREFIX + len);
    buf.put_u32(len as u32);
    envelope.encode(&mut buf).expect("buffer has capacity");
    Ok(buf.freeze())
}

/// Take one envelope off the front of `buf`, bytes read from a stream socket.
///
/// `Ok(None)`: not a whole envelope yet, read more. A length prefix over
/// [`MAX_FRAME`] is an error as soon as the prefix is in, and so is an envelope
/// that does not decode: the stream's boundaries are lost, the connection ends.
pub fn decode_stream(buf: &mut BytesMut) -> Result<Option<Envelope>, CodecError> {
    let Some(len) = stream_frame_len(buf)? else {
        return Ok(None);
    };
    if buf.len() < LENGTH_PREFIX + len {
        buf.reserve(LENGTH_PREFIX + len - buf.len());
        return Ok(None);
    }
    buf.advance(LENGTH_PREFIX);
    let body = buf.split_to(len).freeze();
    Envelope::decode(body).map(Some).map_err(CodecError::Proto)
}

/// The length a stream frame announces, once its prefix is in `buf`.
pub fn stream_frame_len(buf: &[u8]) -> Result<Option<usize>, CodecError> {
    let Some(prefix) = buf.get(..LENGTH_PREFIX) else {
        return Ok(None);
    };
    let len = u32::from_be_bytes(prefix.try_into().expect("four bytes")) as usize;
    if len > MAX_FRAME {
        return Err(CodecError::TooLarge(len));
    }
    Ok(Some(len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{
        Activate, Call, Cancel, Chunk, ConnectionError, CredentialUpdate, Credit, End, Error,
        Event, Hello, HelloAck, Kind, Lane, MethodSpec, Reply, Select, Subscribe, Unsubscribe,
        reply,
    };

    fn env(msg: envelope::Msg) -> Envelope {
        Envelope { msg: Some(msg) }
    }

    /// One envelope of every kind, with every field set.
    fn every_kind() -> Vec<Envelope> {
        use envelope::Msg;
        vec![
            env(Msg::Hello(Hello {
                protocol_version: 1,
                client_name: "test".into(),
                client_id: "6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4".into(),
                access_token: "secret".into(),
                credential: Bytes::from_static(b"\x01\x02"),
                instance_token: "00ff".into(),
                provides: vec![MethodSpec {
                    method: "ui.notify".into(),
                    kind: Kind::Stream as i32,
                    lane: Lane::Bulk as i32,
                    select: Select::LastActive as i32,
                }],
            })),
            env(Msg::HelloAck(HelloAck {
                protocol_version: 1,
                identity: "instance:t-1".into(),
                connection_id: 7,
                max_frame: MAX_FRAME as u32,
                max_in_flight: 256,
                stream_window: 1 << 20,
                connection_window: 1 << 20,
                credential: Bytes::from_static(b"token"),
                server_id: "id".into(),
            })),
            env(Msg::Call(Call {
                id: 1,
                target: "instance:t-2".into(),
                method: "test.echo".into(),
                payload: Bytes::from_static(b"\x00\xff"),
                from: "owner".into(),
                trace_id: "t".into(),
                timeout_ms: 1000,
                content_type: "application/octet-stream".into(),
                credential: Bytes::from_static(b"c"),
                window: 65536,
            })),
            env(Msg::Reply(Reply {
                id: 1,
                result: Some(reply::Result::Payload(Bytes::from_static(b"x"))),
                content_type: "application/json".into(),
            })),
            env(Msg::Reply(Reply {
                id: 2,
                result: Some(reply::Result::Error(Error {
                    code: "not_found".into(),
                    message: "no provider of nobody.here".into(),
                })),
                content_type: String::new(),
            })),
            env(Msg::Cancel(Cancel {
                id: 3,
                reason: "timeout".into(),
            })),
            env(Msg::Event(Event {
                subject: "pane.created".into(),
                payload: Bytes::from_static(b"p"),
                content_type: "application/x-protobuf".into(),
                from: "tilrd".into(),
                scope: "pane:1".into(),
            })),
            env(Msg::Subscribe(Subscribe {
                id: 4,
                pattern: "pane.>".into(),
            })),
            env(Msg::Unsubscribe(Unsubscribe { id: 4 })),
            env(Msg::Chunk(Chunk {
                id: 5,
                payload: Bytes::from_static(b"chunk"),
                content_encoding: ENCODING_ZSTD.into(),
            })),
            env(Msg::End(End {
                id: 5,
                payload: Bytes::from_static(b"end"),
            })),
            env(Msg::Credit(Credit { id: 5, bytes: 4096 })),
            env(Msg::CredentialUpdate(CredentialUpdate {
                credential: Bytes::from_static(b"new"),
            })),
            env(Msg::Activate(Activate {})),
            env(Msg::ConnectionError(ConnectionError {
                code: "expected_hello".into(),
                message: "expected Hello first".into(),
            })),
        ]
    }

    fn large_call(len: usize) -> Envelope {
        env(envelope::Msg::Call(Call {
            id: 9,
            method: "test.echo".into(),
            payload: Bytes::from("hello world ".repeat(len / 12 + 1)),
            ..Default::default()
        }))
    }

    #[test]
    fn every_envelope_kind_round_trips_on_both_transports() {
        let kinds = every_kind();
        assert_eq!(kinds.len(), 15, "one per Envelope variant, Reply twice");
        for envelope in kinds {
            assert_eq!(decode_ws(&encode_ws(&envelope).unwrap()).unwrap(), envelope);
            let mut buf = BytesMut::from(&encode_stream(&envelope).unwrap()[..]);
            assert_eq!(decode_stream(&mut buf).unwrap(), Some(envelope));
            assert!(buf.is_empty());
        }
    }

    #[test]
    fn small_envelopes_stay_plain() {
        let envelope = env(envelope::Msg::Credit(Credit { id: 1, bytes: 10 }));
        let message = encode_ws(&envelope).unwrap();
        assert_eq!(message[0], FLAG_PLAIN);
        assert_eq!(&message[1..], envelope.encode_to_vec().as_slice());
        assert_eq!(decode_ws(&message).unwrap(), envelope);
    }

    #[test]
    fn large_envelopes_are_compressed() {
        let envelope = large_call(4096);
        let message = encode_ws(&envelope).unwrap();
        assert_eq!(message[0], FLAG_ZSTD);
        assert!(message.len() < envelope.encoded_len());
        assert_eq!(decode_ws(&message).unwrap(), envelope);
    }

    fn noise(len: usize) -> Vec<u8> {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect()
    }

    #[test]
    fn a_compressed_chunk_is_not_compressed_again() {
        // The codec takes the producer's word: a chunk marked zstd goes plain even
        // when its bytes (here repetitive) would compress.
        let payload = Bytes::from("frame ".repeat(1000));
        let envelope = env(envelope::Msg::Chunk(Chunk {
            id: 1,
            payload: payload.clone(),
            content_encoding: ENCODING_ZSTD.into(),
        }));
        assert!(envelope.encoded_len() >= COMPRESS_THRESHOLD);
        let message = encode_ws(&envelope).unwrap();
        assert_eq!(message[0], FLAG_PLAIN);
        let Some(envelope::Msg::Chunk(chunk)) = decode_ws(&message).unwrap().msg else {
            panic!("not a chunk");
        };
        assert_eq!(chunk.payload, payload);
    }

    #[test]
    fn an_envelope_that_does_not_compress_goes_plain() {
        let envelope = env(envelope::Msg::Call(Call {
            id: 1,
            method: "test.echo".into(),
            payload: Bytes::from(noise(1 << 20)),
            ..Default::default()
        }));
        let message = encode_ws(&envelope).unwrap();
        assert_eq!(message[0], FLAG_PLAIN);
        assert_eq!(message.len(), envelope.encoded_len() + 1);
        assert_eq!(decode_ws(&message).unwrap(), envelope);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(decode_ws(&[]), Err(CodecError::Empty)));
        assert!(matches!(
            decode_ws(&[9, 1, 2]),
            Err(CodecError::UnknownFlag(9))
        ));
        assert!(matches!(
            decode_ws(&[FLAG_ZSTD, 1, 2, 3]),
            Err(CodecError::Zstd(_))
        ));
        // Field 1 (hello) as a length-delimited field running past the end.
        assert!(matches!(
            decode_ws(&[FLAG_PLAIN, 0x0a, 0x05, 1]),
            Err(CodecError::Proto(_))
        ));
        let mut buf = BytesMut::from(&[0, 0, 0, 3, 0x0a, 0x05, 1][..]);
        assert!(matches!(decode_stream(&mut buf), Err(CodecError::Proto(_))));
    }

    #[test]
    fn a_truncated_stream_frame_waits_for_the_rest() {
        let envelope = large_call(1000);
        let framed = encode_stream(&envelope).unwrap();
        let mut buf = BytesMut::new();
        for (i, byte) in framed.iter().enumerate() {
            assert_eq!(decode_stream(&mut buf).unwrap(), None, "after {i} bytes");
            buf.put_u8(*byte);
        }
        assert_eq!(decode_stream(&mut buf).unwrap(), Some(envelope));
    }

    #[test]
    fn two_stream_frames_in_one_read_come_apart() {
        let first = env(envelope::Msg::Activate(Activate {}));
        let second = large_call(300);
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&encode_stream(&first).unwrap());
        buf.extend_from_slice(&encode_stream(&second).unwrap());
        assert_eq!(decode_stream(&mut buf).unwrap(), Some(first));
        assert_eq!(decode_stream(&mut buf).unwrap(), Some(second));
        assert_eq!(decode_stream(&mut buf).unwrap(), None);
    }

    #[test]
    fn a_frame_over_the_limit_is_refused() {
        let over = large_call(MAX_FRAME + 1);
        assert!(matches!(encode_ws(&over), Err(CodecError::TooLarge(_))));
        assert!(matches!(encode_stream(&over), Err(CodecError::TooLarge(_))));

        // A length prefix of 16 MiB + 1 is refused before its body arrives.
        let mut buf = BytesMut::from(&((MAX_FRAME as u32 + 1).to_be_bytes())[..]);
        assert!(matches!(
            decode_stream(&mut buf),
            Err(CodecError::TooLarge(len)) if len == MAX_FRAME + 1
        ));

        // A plain WebSocket body over the limit.
        let mut message = vec![FLAG_PLAIN];
        message.resize(MAX_FRAME + 2, 0);
        assert!(matches!(decode_ws(&message), Err(CodecError::TooLarge(_))));

        // A small zstd body that decompresses past the limit.
        let bomb = zstd::encode_all(vec![0u8; MAX_FRAME + 1].as_slice(), 3).unwrap();
        assert!(bomb.len() < 1 << 16);
        let mut message = vec![FLAG_ZSTD];
        message.extend_from_slice(&bomb);
        assert!(matches!(decode_ws(&message), Err(CodecError::TooLarge(_))));
    }

    #[test]
    fn a_frame_at_the_limit_passes() {
        let mut envelope = large_call(0);
        let overhead = envelope.encoded_len() + 8;
        let Some(envelope::Msg::Call(call)) = &mut envelope.msg else {
            unreachable!()
        };
        call.payload = Bytes::from(vec![b'x'; MAX_FRAME - overhead]);
        // Fill up to exactly MAX_FRAME by growing the payload byte by byte.
        while envelope.encoded_len() < MAX_FRAME {
            let Some(envelope::Msg::Call(call)) = &mut envelope.msg else {
                unreachable!()
            };
            call.payload = Bytes::from(vec![b'x'; call.payload.len() + 1]);
        }
        assert_eq!(envelope.encoded_len(), MAX_FRAME);
        let framed = encode_stream(&envelope).unwrap();
        let mut buf = BytesMut::from(&framed[..]);
        assert_eq!(decode_stream(&mut buf).unwrap(), Some(envelope.clone()));
        assert_eq!(decode_ws(&encode_ws(&envelope).unwrap()).unwrap(), envelope);
    }

    #[test]
    fn unknown_envelope_variants_are_ignored() {
        // Field 99 (an envelope variant of a later version), varint 1, then an
        // unknown field inside a known variant.
        let message = [FLAG_PLAIN, 0x98, 0x06, 0x01];
        assert_eq!(decode_ws(&message).unwrap(), Envelope { msg: None });

        let mut known = env(envelope::Msg::Credit(Credit { id: 1, bytes: 2 })).encode_to_vec();
        // Inside Credit (field 11, length-delimited): append field 15 varint 7.
        known[1] += 2;
        known.extend_from_slice(&[0x78, 0x07]);
        let mut message = vec![FLAG_PLAIN];
        message.extend_from_slice(&known);
        assert_eq!(
            decode_ws(&message).unwrap(),
            env(envelope::Msg::Credit(Credit { id: 1, bytes: 2 }))
        );
    }
}
