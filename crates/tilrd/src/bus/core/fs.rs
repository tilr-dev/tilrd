//! `fs.*` (T2), feature `files`: `fs.list`, `fs.stat`, `fs.read` (a bulk
//! stream), `fs.write` (one call per chunk), `fs.mkdir`, `fs.rename`,
//! `fs.delete`, `fs.watch` with the events `fs.changed.<watch_id>`, and
//! `fs.unwatch`.
//!
//! Owner: `bus-core-services`.
//!
//! What touches the disk is `crate::files::Access`, the same roots, read-only
//! mode and limits as `tilr.v1`'s files; the supervisor's task only hands it out
//! and keeps what a caller leaves behind ([`FsState`]): its watches and the
//! uploads in progress, dropped when its connection ends. Each chunk of an
//! upload is answered once it is written (the last once the file is replaced),
//! so an upload is paced by its replies.

use std::collections::HashMap;
use std::ffi::OsString;

use bytes::Bytes;
use prost::Message;
use tilr_bus::bus::Lane;
use tilr_bus::fs::{
    DeleteRequest, FileKind, FsAck, FsChange, FsChanged, FsData, FsEntries, FsEntry, FsInfo,
    ListRequest, MkdirRequest, ReadRequest, RenameRequest, StatRequest, UnwatchRequest, WatchReply,
    WatchRequest, WriteRequest,
};
use tilr_bus::{BusError, codes, methods};
use tokio::sync::{mpsc, oneshot};

use super::json;
use super::server::{asked, conn_of, json_message, on_connection_end, queue, refused, unary};
use super::{Core, Registry, SupervisorHandle};
use crate::bus::identity::{CallCtx, Identity};
use crate::bus::streams::{Chunk, NativeMethod, StreamOut};
use crate::files::{self, Access, Change, ChangeKind, Entry, Kind, Upload, Watch, WriteStart};
use crate::supervisor::Supervisor;

json_message!(ListRequest {
    path: string "path",
    path_bytes: bytes "pathBytes",
});
json_message!(StatRequest {
    path: string "path",
    path_bytes: bytes "pathBytes",
});
json_message!(ReadRequest {
    path: string "path",
    offset: u64 "offset",
    length: u64 "length",
    path_bytes: bytes "pathBytes",
});
json_message!(WriteRequest {
    transfer_id: u64 "transferId",
    path: string "path",
    total_size: u64 "totalSize",
    create: bool "create",
    truncate: bool "truncate",
    data: bytes "data",
    last: bool "last",
    path_bytes: bytes "pathBytes",
});
json_message!(MkdirRequest {
    path: string "path",
    path_bytes: bytes "pathBytes",
});
json_message!(RenameRequest {
    from: string "from",
    to: string "to",
    from_bytes: bytes "fromBytes",
    to_bytes: bytes "toBytes",
});
json_message!(DeleteRequest {
    path: string "path",
    recursive: bool "recursive",
    path_bytes: bytes "pathBytes",
});
json_message!(WatchRequest {
    path: string "path",
    path_bytes: bytes "pathBytes",
});
json_message!(WatchReply {
    watch_id: u64 "watchId",
});
json_message!(UnwatchRequest {
    watch_id: u64 "watchId",
});
json_message!(FsAck {});
json_message!(FsEntry {
    name: string "name",
    kind: enumeration(FileKind) "kind",
    size: u64 "size",
    mtime_ms: u64 "mtimeMs",
    mode: u32 "mode",
    target: enumeration(FileKind) "target",
    name_bytes: bytes "nameBytes",
});
json_message!(FsEntries {
    path: string "path",
    entries: messages(FsEntry) "entries",
    path_bytes: bytes "pathBytes",
});
json_message!(FsInfo {
    path: string "path",
    entry: message(FsEntry) "entry",
    path_bytes: bytes "pathBytes",
});

/// A chunk of an upload, and who waits for its answer.
type Piece = (WriteRequest, oneshot::Sender<Result<(), String>>);

/// What this module keeps on the supervisor (`CoreState::slot`).
#[derive(Default)]
struct FsState {
    next_watch: u64,
    /// The watches, by id, with the connection that made each.
    watches: HashMap<u64, (u64, Watch)>,
    /// Uploads in progress, by connection and transfer.
    writes: HashMap<(u64, u64), mpsc::UnboundedSender<Piece>>,
}

fn state(s: &mut Supervisor) -> &mut FsState {
    s.bus.slot::<FsState>()
}

/// Add `fs.*` when `core.features.files`.
pub fn register(registry: &mut Registry, core: &Core) {
    if !core.features.files {
        return;
    }
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::FS_LIST.name,
        move |_: &CallCtx, request: ListRequest| {
            let access = access(&supervisor);
            async move {
                let access = asked(access?).await??;
                let path = files::requested(&request.path, &request.path_bytes);
                let (dir, entries) = blocking(move || access.list(&path)).await?;
                let (path, path_bytes) = files::name_of(dir.as_os_str());
                Ok(FsEntries {
                    path,
                    entries: entries.into_iter().map(entry).collect(),
                    path_bytes: path_bytes.into(),
                })
            }
        },
    ));
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::FS_STAT.name,
        move |_: &CallCtx, request: StatRequest| {
            let access = access(&supervisor);
            async move {
                let access = asked(access?).await??;
                let path = files::requested(&request.path, &request.path_bytes);
                let (real, found) = blocking(move || access.stat(&path)).await?;
                let (path, path_bytes) = files::name_of(real.as_os_str());
                Ok(FsInfo {
                    path,
                    entry: Some(entry(found)),
                    path_bytes: path_bytes.into(),
                })
            }
        },
    ));
    let reader = core.supervisor.clone();
    registry.add(NativeMethod::stream(
        methods::FS_READ.name,
        Lane::Bulk,
        move |ctx, payload, out| read(reader.clone(), ctx, payload, out),
    ));
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::FS_WRITE.name,
        move |ctx: &CallCtx, request: WriteRequest| {
            let conn = conn_of(ctx);
            let (tx, rx) = oneshot::channel();
            // Queued now: the chunks of one caller keep their order.
            let queued = supervisor.with(move |s| write(s, conn, request, tx));
            async move {
                queued?;
                rx.await
                    .map_err(|_| BusError::unavailable("the upload was dropped"))?
                    .map_err(refused)?;
                Ok(FsAck {})
            }
        },
    ));
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::FS_MKDIR.name,
        move |_: &CallCtx, request: MkdirRequest| {
            let access = access(&supervisor);
            async move {
                let access = asked(access?).await??;
                let path = files::requested(&request.path, &request.path_bytes);
                blocking(move || access.mkdir(&path)).await?;
                Ok(FsAck {})
            }
        },
    ));
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::FS_RENAME.name,
        move |_: &CallCtx, request: RenameRequest| {
            let access = access(&supervisor);
            async move {
                let access = asked(access?).await??;
                let from = files::requested(&request.from, &request.from_bytes);
                let to = files::requested(&request.to, &request.to_bytes);
                blocking(move || access.rename(&from, &to)).await?;
                Ok(FsAck {})
            }
        },
    ));
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::FS_DELETE.name,
        move |_: &CallCtx, request: DeleteRequest| {
            let access = access(&supervisor);
            async move {
                let access = asked(access?).await??;
                let path = files::requested(&request.path, &request.path_bytes);
                blocking(move || access.delete(&path, request.recursive)).await?;
                Ok(FsAck {})
            }
        },
    ));
    let watcher = core.clone();
    registry.add(unary(
        methods::FS_WATCH.name,
        move |ctx: &CallCtx, request: WatchRequest| watch(watcher.clone(), conn_of(ctx), request),
    ));
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::FS_UNWATCH.name,
        move |ctx: &CallCtx, request: UnwatchRequest| {
            let conn = conn_of(ctx);
            let removed = queue(&supervisor, move |s| {
                let st = state(s);
                match st.watches.get(&request.watch_id) {
                    Some((owner, _)) if *owner == conn => {
                        st.watches.remove(&request.watch_id);
                        Ok(FsAck {})
                    }
                    _ => Err(BusError::new(
                        codes::NOT_FOUND,
                        format!("no watch {} of this connection", request.watch_id),
                    )),
                }
            });
            async move { asked(removed?).await? }
        },
    ));
}

/// The way to the files, from the supervisor.
fn access(
    supervisor: &SupervisorHandle,
) -> Result<oneshot::Receiver<Result<Access, BusError>>, BusError> {
    queue(supervisor, |s| {
        s.files
            .as_ref()
            .map(files::Files::access)
            .ok_or_else(|| BusError::unavailable("files are turned off on this server"))
    })
}

/// Run what reads the disk where that may block.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, BusError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| BusError::handler_error(err.to_string()))?
        .map_err(refused)
}

fn kind(kind: Kind) -> FileKind {
    match kind {
        Kind::File => FileKind::File,
        Kind::Dir => FileKind::Dir,
        Kind::Link => FileKind::Link,
        Kind::Other => FileKind::Other,
    }
}

fn entry(entry: Entry) -> FsEntry {
    let (name, name_bytes) = files::name_of(&entry.name);
    FsEntry {
        name,
        kind: kind(entry.kind).into(),
        size: entry.size,
        mtime_ms: entry.mtime_ms,
        mode: entry.mode,
        target: kind(entry.target).into(),
        name_bytes: name_bytes.into(),
    }
}

/// The payload room a chunk leaves in the stream's window once `empty` (the
/// chunk's message without its data) is encoded: what the data may take.
pub(super) fn room(window: u64, empty: usize) -> usize {
    // The data's tag and length.
    let overhead = empty + 6;
    usize::try_from(window)
        .unwrap_or(usize::MAX)
        .saturating_sub(overhead)
        .max(1)
}

/// `fs.read`: the file's part in chunks of at most 256 KiB, split further to
/// the stream's window; a failure ends the stream with it.
async fn read(supervisor: SupervisorHandle, ctx: CallCtx, payload: Bytes, out: StreamOut) {
    let result = async {
        let request = json::decode::<ReadRequest>(&ctx, &payload)?;
        let access = asked(access(&supervisor)?).await??;
        let path = files::requested(&request.path, &request.path_bytes);
        let mut reader = access
            .open_read(path, request.offset, request.length)
            .await
            .map_err(refused)?;
        let empty = FsData {
            total_size: reader.total,
            data: Bytes::new(),
            file_size: reader.file_size,
        };
        let room = room(out.window(), empty.encoded_len());
        while let Some(data) = reader.next().await.map_err(refused)? {
            for piece in split(data, room) {
                let chunk = FsData {
                    data: piece,
                    ..empty.clone()
                };
                if out.send(Chunk::new(chunk.encode_to_vec())).await.is_err() {
                    return Ok(());
                }
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

/// `data` in pieces of at most `room` bytes; one empty piece for no data.
pub(super) fn split(data: Bytes, room: usize) -> Vec<Bytes> {
    if data.len() <= room {
        return vec![data];
    }
    (0..data.len())
        .step_by(room)
        .map(|at| data.slice(at..data.len().min(at + room)))
        .collect()
}

/// A chunk of `fs.write`, on the supervisor's task: to the upload of its
/// transfer, started by the first.
fn write(
    s: &mut Supervisor,
    conn: u64,
    chunk: WriteRequest,
    reply: oneshot::Sender<Result<(), String>>,
) {
    let Some(access) = s.files.as_ref().map(files::Files::access) else {
        let _ = reply.send(Err("files are turned off on this server".into()));
        return;
    };
    on_connection_end(s, "fs", conn, forget_connection);
    let key = (conn, chunk.transfer_id);
    let last = chunk.last;
    let st = state(s);
    let pieces = st.writes.entry(key).or_insert_with(|| {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(upload(access, rx));
        tx
    });
    let _ = pieces.send((chunk, reply));
    if last {
        st.writes.remove(&key);
    }
}

/// One upload: each chunk answered once written; once one failed, it and
/// every later one are answered with that failure, and the file stays as it
/// was. Ends after the last chunk, or when the caller's connection ends.
async fn upload(access: Access, mut pieces: mpsc::UnboundedReceiver<Piece>) {
    let mut upload: Result<Option<Upload>, String> = Ok(None);
    while let Some((chunk, reply)) = pieces.recv().await {
        let last = chunk.last;
        let result = match &mut upload {
            Err(failed) => Err(failed.clone()),
            Ok(open) => take(&access, open, chunk).await,
        };
        if let Err(failed) = &result {
            upload = Err(failed.clone());
        }
        let _ = reply.send(result);
        if last {
            return;
        }
    }
}

async fn take(
    access: &Access,
    open: &mut Option<Upload>,
    chunk: WriteRequest,
) -> Result<(), String> {
    let upload = match open {
        Some(upload) => upload,
        None => open.insert(
            access
                .begin_write(WriteStart {
                    path: files::requested(&chunk.path, &chunk.path_bytes),
                    total_size: chunk.total_size,
                    create: chunk.create,
                    truncate: chunk.truncate,
                })
                .await?,
        ),
    };
    upload.take(&chunk.data).await?;
    if chunk.last
        && let Some(upload) = open.take()
    {
        upload.finish().await?;
    }
    Ok(())
}

/// The caller's connection ended: its uploads are abandoned and its watches
/// stop.
fn forget_connection(s: &mut Supervisor, conn: u64) {
    let st = state(s);
    st.writes.retain(|&(owner, _), _| owner != conn);
    st.watches.retain(|_, (owner, _)| *owner != conn);
}

/// `fs.watch`: answered once the server watches.
fn watch(
    core: Core,
    conn: u64,
    request: WatchRequest,
) -> impl Future<Output = Result<WatchReply, BusError>> + Send + 'static {
    let begun = queue(&core.supervisor, move |s| {
        let access = s.files.as_ref().map(files::Files::access);
        on_connection_end(s, "fs", conn, forget_connection);
        let st = state(s);
        st.next_watch += 1;
        (access, st.next_watch)
    });
    async move {
        let (access, watch_id) = asked(begun?).await?;
        let access =
            access.ok_or_else(|| BusError::unavailable("files are turned off on this server"))?;
        let path: OsString = files::requested(&request.path, &request.path_bytes);
        let broker = core.broker.clone();
        let subject = format!("fs.changed.{watch_id}");
        let sink = std::sync::Arc::new(move |change: Change| {
            let (path, path_bytes) = files::name_of(&change.path);
            let event = FsChanged {
                watch: change.watch,
                path,
                kind: match change.kind {
                    ChangeKind::Create => FsChange::Create,
                    ChangeKind::Modify => FsChange::Modify,
                    ChangeKind::Remove => FsChange::Remove,
                }
                .into(),
                path_bytes: path_bytes.into(),
                watch_id,
            };
            broker.emit(
                Identity::Tilrd,
                &subject,
                event.encode_to_vec().into(),
                None,
            );
        });
        let named = request.path.clone();
        let handle = blocking(move || access.watch(&path, &named, sink)).await?;
        let kept = queue(&core.supervisor, move |s| {
            // A connection that ended meanwhile gets no watch.
            let alive = s
                .bus
                .broker()
                .is_some_and(|broker| broker.principal_of(conn).is_some());
            if alive {
                state(s).watches.insert(watch_id, (conn, handle));
            }
            alive
        });
        if !asked(kept?).await? {
            return Err(BusError::unavailable("the connection ended"));
        }
        Ok(WatchReply { watch_id })
    }
}
