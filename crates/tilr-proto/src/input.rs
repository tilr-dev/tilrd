//! Input retained for the lifetime of a client session, across connections.
use std::collections::{HashMap, VecDeque};

use prost::Message;

use crate::{ClientMessage, InputAck, client_message::Msg};

const MAX_BYTES: usize = 4 << 20;
const MAX_FRAMES: usize = 4096;

pub fn pane(msg: &Msg) -> Option<&str> {
    match msg {
        Msg::Key(m) => Some(&m.pane_id),
        Msg::Mouse(m) => Some(&m.pane_id),
        Msg::Paste(m) => Some(&m.pane_id),
        Msg::Raw(m) => Some(&m.pane_id),
        Msg::Scroll(m) => Some(&m.pane_id),
        Msg::Resize(m) => Some(&m.pane_id),
        _ => None,
    }
}

#[derive(Default)]
pub struct Outbox {
    next: HashMap<String, u64>,
    queued: VecDeque<ClientMessage>,
    bytes: usize,
}

impl Outbox {
    /// Admission is atomic: an overflow consumes neither a sequence nor capacity.
    pub fn push(&mut self, mut message: ClientMessage) -> Result<ClientMessage, &'static str> {
        let pane = message
            .msg
            .as_ref()
            .and_then(pane)
            .ok_or("not pane input")?
            .to_owned();
        message.input_seq = self
            .next
            .get(&pane)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("input sequence exhausted")?;
        let size = message.encoded_len();
        if self.queued.len() == MAX_FRAMES || size > MAX_BYTES.saturating_sub(self.bytes) {
            return Err("input buffer full: input was not accepted");
        }
        self.next.insert(pane, message.input_seq);
        self.bytes += size;
        self.queued.push_back(message.clone());
        Ok(message)
    }

    pub fn acknowledge(&mut self, ack: &InputAck) {
        // Also seed the counter when resuming an identity with an empty queue.
        let next = self.next.entry(ack.pane_id.clone()).or_default();
        *next = (*next).max(ack.seq);
        self.queued.retain(|message| {
            let keep = message.input_seq > ack.seq
                || message.msg.as_ref().and_then(pane) != Some(ack.pane_id.as_str());
            if !keep {
                self.bytes -= message.encoded_len();
            }
            keep
        });
    }

    pub fn pending(&self) -> impl Iterator<Item = &ClientMessage> {
        self.queued.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw(pane: &str, data: Vec<u8>) -> ClientMessage {
        crate::client_message::Msg::Raw(crate::RawInput {
            pane_id: pane.into(),
            data: data.into(),
        })
        .into()
    }

    #[test]
    fn replay_retains_order_and_acknowledgements_are_per_pane() {
        let mut queue = Outbox::default();
        assert_eq!(queue.push(raw("a", vec![1])).unwrap().input_seq, 1);
        assert_eq!(queue.push(raw("b", vec![2])).unwrap().input_seq, 1);
        assert_eq!(queue.push(raw("a", vec![3])).unwrap().input_seq, 2);
        queue.acknowledge(&InputAck {
            pane_id: "a".into(),
            seq: 1,
        });
        assert_eq!(
            queue
                .pending()
                .map(|m| (pane(m.msg.as_ref().unwrap()).unwrap(), m.input_seq))
                .collect::<Vec<_>>(),
            [("b", 1), ("a", 2)]
        );
        queue.acknowledge(&InputAck {
            pane_id: "a".into(),
            seq: 2,
        });
        queue.acknowledge(&InputAck {
            pane_id: "a".into(),
            seq: 1,
        });
        assert_eq!(queue.push(raw("a", vec![4])).unwrap().input_seq, 3);
    }

    #[test]
    fn overflow_is_atomic_and_ack_reclaims_capacity() {
        let mut queue = Outbox::default();
        assert!(queue.push(raw("a", vec![0; MAX_BYTES])).is_err());
        for expected in 1..=MAX_FRAMES as u64 {
            assert_eq!(queue.push(raw("a", vec![])).unwrap().input_seq, expected);
        }
        assert!(queue.push(raw("a", vec![])).is_err());
        assert_eq!(queue.pending().count(), MAX_FRAMES);
        queue.acknowledge(&InputAck {
            pane_id: "a".into(),
            seq: 1,
        });
        assert_eq!(
            queue.push(raw("a", vec![])).unwrap().input_seq,
            MAX_FRAMES as u64 + 1
        );
    }
}
