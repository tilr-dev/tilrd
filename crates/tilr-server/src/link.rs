//! Packet framing between the supervisor and a pane process, over the pane
//! process's stdin and stdout: u32 LE length, then a `tilr_proto` packet.
//!
//! An empty packet from the supervisor is not a message: it asks the pane
//! process to replace itself with the executable currently on disk.

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_PACKET: usize = 64 << 20;

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
    fn take_packet_rejects_an_oversized_length() {
        let mut buf = BytesMut::from(&u32::MAX.to_le_bytes()[..]);
        assert!(take_packet(&mut buf).is_err());
    }
}
