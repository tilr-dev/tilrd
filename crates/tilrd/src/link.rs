//! Packet framing between the supervisor and a pane process, over the pane
//! process's stdin and stdout: u32 LE length, then a `tilr_proto` packet.
//!
//! An empty packet from the supervisor is not a message: it asks the pane
//! process to replace itself with the executable currently on disk.

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest packet either side takes.
pub const MAX_PACKET: usize = 64 << 20;

/// Read one packet. `Ok(None)` means the peer closed the socket cleanly.
pub async fn read_packet<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<Option<Bytes>> {
    let mut len = [0u8; 4];
    match reader.read_exact(&mut len).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_PACKET {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("packet of {len} bytes exceeds limit"),
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Some(Bytes::from(body)))
}

/// Take one whole packet off the front of `buf`, if it holds one. For a
/// reader that must be able to stop between packets: unlike [`read_packet`],
/// what was read of an unfinished packet stays in `buf`.
pub fn take_packet(buf: &mut BytesMut) -> std::io::Result<Option<Bytes>> {
    let Some(len) = buf.first_chunk::<4>() else {
        return Ok(None);
    };
    let len = u32::from_le_bytes(*len) as usize;
    if len > MAX_PACKET {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("packet of {len} bytes exceeds limit"),
        ));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    buf.advance(4);
    Ok(Some(buf.split_to(len).freeze()))
}

/// First byte of every packet a pane process sends to the supervisor.
pub mod kind {
    /// A `ServerMessage` packet to relay to the pane's subscribers untouched.
    pub const RELAY: u8 = 0;
    /// A `PaneStatus` packet for the supervisor itself.
    pub const STATUS: u8 = 1;
    /// The shell is running. No body.
    pub const STARTED: u8 = 2;
    /// A `PaneExited` packet: the shell exited and the process is about to.
    pub const EXITED: u8 = 3;
    /// An `Error` packet: the shell could not be started.
    pub const FAILED: u8 = 4;
    /// A frame to relay, behind a [`FrameHead`]: what the supervisor needs
    /// to know of it without decoding it. A pane process sends its frames
    /// this way; `RELAY` is what one from before it did.
    pub const FRAME: u8 = 5;
    /// A `ClipboardWrite` packet, whole (`pane_id` and `transfer_id` left
    /// empty): the program in the pane copied this to the clipboard.
    pub const CLIPBOARD: u8 = 6;
    /// A `PaneActivity` packet to broadcast to every greeted client.
    pub const ACTIVITY: u8 = 7;
    /// A private PaneInputAck, routed only to its authenticated identity.
    pub const INPUT_ACK: u8 = 8;
}

/// What a `FRAME` packet says of its frame ahead of the `ServerMessage`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameHead {
    /// `Frame.full`.
    pub full: bool,
    /// `Frame.last_input_seq`.
    pub last_input_seq: u64,
}

impl FrameHead {
    const LEN: usize = 9;

    /// The head in front of `packet`, as the body of a `FRAME` packet.
    pub fn prepend(self, packet: &[u8]) -> Vec<u8> {
        let mut body = Vec::with_capacity(Self::LEN + packet.len());
        body.push(u8::from(self.full));
        body.extend_from_slice(&self.last_input_seq.to_le_bytes());
        body.extend_from_slice(packet);
        body
    }

    /// Split the body of a `FRAME` packet into its head and the packet.
    pub fn split(mut body: Bytes) -> Option<(Self, Bytes)> {
        if body.len() < Self::LEN {
            return None;
        }
        let head = body.split_to(Self::LEN);
        let seq = head[1..].try_into().ok()?;
        Some((
            Self {
                full: head[0] & 1 != 0,
                last_input_seq: u64::from_le_bytes(seq),
            },
            body,
        ))
    }
}

/// Like [`write_packet`], with a [`kind`] byte in front of the packet.
pub async fn write_tagged<W: AsyncWrite + Unpin>(
    writer: &mut W,
    kind: u8,
    packet: &[u8],
) -> std::io::Result<()> {
    writer
        .write_all(&(packet.len() as u32 + 1).to_le_bytes())
        .await?;
    writer.write_all(&[kind]).await?;
    writer.write_all(packet).await?;
    writer.flush().await
}

pub async fn write_packet<W: AsyncWrite + Unpin>(
    writer: &mut W,
    packet: &[u8],
) -> std::io::Result<()> {
    writer
        .write_all(&(packet.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(packet).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_packet_waits_for_the_whole_packet() {
        let mut wire = Vec::new();
        for packet in [&b"one"[..], b"", b"three"] {
            wire.extend_from_slice(&(packet.len() as u32).to_le_bytes());
            wire.extend_from_slice(packet);
        }
        // Feed the stream a byte at a time: packets come out whole, in order.
        let mut buf = BytesMut::new();
        let mut packets = Vec::new();
        for byte in wire {
            buf.extend_from_slice(&[byte]);
            while let Some(packet) = take_packet(&mut buf).unwrap() {
                packets.push(packet);
            }
        }
        assert_eq!(packets, [&b"one"[..], b"", b"three"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn a_frame_head_comes_off_as_it_went_on() {
        let head = FrameHead {
            full: true,
            last_input_seq: (7 << 32) + 3,
        };
        let body = Bytes::from(head.prepend(b"frame"));
        assert_eq!(FrameHead::split(body), Some((head, Bytes::from("frame"))));
        assert_eq!(FrameHead::split(Bytes::from("short")), None);
    }

    #[test]
    fn take_packet_rejects_an_oversized_length() {
        let mut buf = BytesMut::from(&u32::MAX.to_le_bytes()[..]);
        assert!(take_packet(&mut buf).is_err());
    }
}
