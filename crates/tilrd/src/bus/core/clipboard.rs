//! `clipboard.*` (T2, T6), feature `clipboard`: `clipboard.put` (one call per
//! chunk), `clipboard.get` (a bulk stream), and the event `pane.copied`.
//!
//! Owner: `bus-core-services`.
//!
//! An upload goes to the server's clipboard store as `tilr.v1`'s
//! `ClipboardPut` does (`crate::clipboard::Receiver`), each chunk answered; the
//! last once the panes' `xclip` and `wl-paste` read it. A copy made in a pane
//! (OSC 52, or `xclip`/`wl-copy` there) is told by the supervisor's hook
//! [`copied`] (in `Supervisor::send_copy`, beside the `tilr.v1`
//! `ClipboardWrite`s): it is kept here as the latest copy and announced as
//! `pane.copied {pane_id, transfer_id, mime, total_size}` with scope
//! `pane:<id>`; `clipboard.get {transfer_id}` streams it, to a caller that may
//! access the pane, until a newer copy replaces it, then answers `not_found`
//! (as it does to one that may not).

use bytes::Bytes;
use prost::Message;
use tilr_bus::bus::Lane;
use tilr_bus::clipboard::{ClipboardData, GetRequest, PutReply, PutRequest};
use tilr_bus::pane::Copied;
use tilr_bus::{BusError, codes, methods};

use super::common::{
    asked, conn_of, json_message, on_connection_end, queue, refused, room, split, unary,
};
use super::json;
use super::{Core, Registry, SupervisorHandle};
use crate::bus::identity::{CallCtx, Identity};
use crate::bus::streams::{Chunk, NativeMethod, StreamOut};
use crate::clipboard::{Put, Taken};
use crate::supervisor::{PaneId, Supervisor};

json_message!(PutRequest {
    transfer_id: u64 "transferId",
    mime: string "mime",
    total_size: u64 "totalSize",
    data: bytes "data",
    last: bool "last",
});
json_message!(PutReply {
    transfer_id: u64 "transferId",
});
json_message!(GetRequest {
    transfer_id: u64 "transferId",
});

/// The event of a copy made in a pane.
pub const PANE_COPIED: &str = "pane.copied";

/// The latest copy made in a pane (`CoreState::slot`).
#[derive(Default)]
struct Latest(Option<PaneCopy>);

#[derive(Clone)]
struct PaneCopy {
    transfer_id: u64,
    pane_id: String,
    mime: String,
    data: Bytes,
}

/// Hook of `bus-core-services` (`Supervisor::send_copy`): a copy made in
/// `pane_id`, `transfer_id` larger than those before. Kept for
/// `clipboard.get` and told as `pane.copied`, with scope `pane:<id>`.
pub(crate) fn copied(
    s: &mut Supervisor,
    pane_id: PaneId,
    transfer_id: u64,
    mime: &str,
    data: &Bytes,
) {
    let Some(broker) = s.bus.broker().cloned() else {
        return;
    };
    let event = Copied {
        pane_id: pane_id.to_string(),
        transfer_id,
        mime: mime.to_owned(),
        total_size: data.len() as u64,
    };
    s.bus.slot::<Latest>().0 = Some(PaneCopy {
        transfer_id,
        pane_id: event.pane_id.clone(),
        mime: event.mime.clone(),
        data: data.clone(),
    });
    broker.emit(
        Identity::Tilrd,
        PANE_COPIED,
        event.encode_to_vec().into(),
        Some(format!("pane:{pane_id}")),
    );
}

/// Add `clipboard.*` when `core.features.clipboard`.
pub fn register(registry: &mut Registry, core: &Core) {
    if !core.features.clipboard {
        return;
    }
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::CLIPBOARD_PUT.name,
        move |ctx: &CallCtx, request: PutRequest| {
            let conn = conn_of(ctx);
            let transfer_id = request.transfer_id;
            // Queued now: the chunks of one caller keep their order.
            let taken = queue(&supervisor, move |s| {
                if !on_connection_end(s, "clipboard", conn, forget_connection) {
                    return Taken::Refused("the connection ended".into());
                }
                let put = Put {
                    transfer_id: request.transfer_id,
                    mime: request.mime,
                    total_size: request.total_size,
                    data: request.data,
                    last: request.last,
                };
                match &mut s.clipboard {
                    Some(clipboard) => clipboard.put(conn, put),
                    None => Taken::Refused("the clipboard is turned off on this server".into()),
                }
            });
            async move {
                match asked(taken?).await? {
                    Taken::Partial | Taken::Done => Ok(PutReply { transfer_id }),
                    Taken::Refused(why) | Taken::StillRefused(why) => {
                        tracing::warn!(conn, "clipboard upload refused: {why}");
                        Err(refused(why))
                    }
                }
            }
        },
    ));
    let getter = core.supervisor.clone();
    registry.add(NativeMethod::stream(
        methods::CLIPBOARD_GET.name,
        Lane::Bulk,
        move |ctx, payload, out| get(getter.clone(), ctx, payload, out),
    ));
}

/// The caller's connection ended: its upload in progress is void.
fn forget_connection(s: &mut Supervisor, conn: u64) {
    if let Some(clipboard) = &mut s.clipboard {
        clipboard.drop_client(conn);
    }
}

/// `clipboard.get`: the copy's content in chunks of at most 256 KiB (split
/// further to the stream's window), `pane_id`, `mime` and `total_size` on the
/// first.
async fn get(supervisor: SupervisorHandle, ctx: CallCtx, payload: Bytes, out: StreamOut) {
    let result = async {
        let request = json::decode::<GetRequest>(&ctx, &payload)?;
        let caller = super::panes::Caller::of(&ctx);
        let latest = queue(&supervisor, move |s| {
            let copy = s.bus.slot::<Latest>().0.clone()?;
            // A copy of a pane the caller may not access is answered as none.
            let pane_id = copy.pane_id.parse().ok()?;
            super::panes::may_access(s, &caller, pane_id).ok()?;
            Some(copy)
        })?;
        let copy = asked(latest)
            .await?
            .filter(|copy| copy.transfer_id == request.transfer_id)
            .ok_or_else(|| {
                BusError::new(
                    codes::NOT_FOUND,
                    format!(
                        "no copy {}: a newer one replaced it, or there was none",
                        request.transfer_id
                    ),
                )
            })?;
        let first = ClipboardData {
            transfer_id: copy.transfer_id,
            pane_id: copy.pane_id,
            mime: copy.mime,
            total_size: copy.data.len() as u64,
            data: Bytes::new(),
        };
        let first_room = room(out.window(), first.encoded_len());
        let mut pieces = Vec::new();
        let mut rest = copy.data;
        let head = rest.split_to(rest.len().min(first_room.min(crate::files::CHUNK)));
        pieces.push(ClipboardData {
            data: head,
            ..first.clone()
        });
        let later = ClipboardData {
            transfer_id: copy.transfer_id,
            ..ClipboardData::default()
        };
        let room = room(out.window(), later.encoded_len()).min(crate::files::CHUNK);
        if !rest.is_empty() {
            for data in split(rest, room) {
                pieces.push(ClipboardData {
                    data,
                    ..later.clone()
                });
            }
        }
        for piece in pieces {
            if out.send(Chunk::new(piece.encode_to_vec())).await.is_err() {
                return Ok(());
            }
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => out.end(Bytes::new()),
        Err(err) => out.fail(err),
    }
}
