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

use super::common::{
    asked, conn_of, json_message, limit, on_connection_end, queue, refused, room, split, unary,
};
use super::json;
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

/// Watches one connection may have at once (each is a watcher of the
/// system's: an inotify instance on Linux, an FSEvents stream on macOS).
pub const MAX_WATCHES_PER_CONNECTION: usize = 32;
/// Uploads one connection may have in progress at once (each holds a
/// temporary file open).
pub const MAX_UPLOADS_PER_CONNECTION: usize = 16;

/// A chunk of an upload, and who waits for its answer.
type Piece = (WriteRequest, oneshot::Sender<Result<(), BusError>>);

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
    #[cfg(feature = "failpoints")]
    {
        let handle = core.supervisor.clone();
        let _ = core.supervisor.with(move |s| {
            s.bus.slot::<LateWrites>().0 = Some(handle);
        });
    }
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
                    .map_err(|_| BusError::unavailable("the upload was dropped"))??;
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

/// A chunk of `fs.write`, on the supervisor's task: to the upload of its
/// transfer, started by the first. A first chunk of a connection that has
/// ended, or that has [`MAX_UPLOADS_PER_CONNECTION`] uploads in progress, is
/// refused and starts nothing.
fn write(
    s: &mut Supervisor,
    conn: u64,
    chunk: WriteRequest,
    reply: oneshot::Sender<Result<(), BusError>>,
) {
    // A test holds the chunk here until the caller's connection has ended.
    #[cfg(feature = "failpoints")]
    if crate::fail::armed(WRITE_LATE)
        && let Some(supervisor) = s.bus.slot::<LateWrites>().0.clone()
    {
        tokio::spawn(async move {
            crate::fail::point_async(WRITE_LATE.into()).await;
            let _ = supervisor.with(move |s| write_now(s, conn, chunk, reply));
        });
        return;
    }
    write_now(s, conn, chunk, reply);
}

/// The failpoint that holds a chunk of `fs.write` before the supervisor
/// takes it.
#[cfg(feature = "failpoints")]
const WRITE_LATE: &str = "bus-fs-write-late";

/// The supervisor's handle, for a chunk held by [`WRITE_LATE`].
#[cfg(feature = "failpoints")]
#[derive(Default)]
struct LateWrites(Option<SupervisorHandle>);

fn write_now(
    s: &mut Supervisor,
    conn: u64,
    chunk: WriteRequest,
    reply: oneshot::Sender<Result<(), BusError>>,
) {
    let Some(access) = s.files.as_ref().map(files::Files::access) else {
        let _ = reply.send(Err(BusError::unavailable(
            "files are turned off on this server",
        )));
        return;
    };
    let key = (conn, chunk.transfer_id);
    let last = chunk.last;
    if !state(s).writes.contains_key(&key) {
        if !on_connection_end(s, "fs", conn, forget_connection) {
            tracing::info!(conn, "fs.write refused: the connection ended");
            let _ = reply.send(Err(BusError::unavailable("the connection ended")));
            return;
        }
        let open = state(s)
            .writes
            .keys()
            .filter(|(owner, _)| *owner == conn)
            .count();
        if open >= MAX_UPLOADS_PER_CONNECTION {
            let _ = reply.send(Err(limit(format!(
                "{MAX_UPLOADS_PER_CONNECTION} uploads in progress already"
            ))));
            return;
        }
        if last {
            // One chunk: nothing to keep between calls.
            tokio::spawn(async move {
                let (tx, rx) = mpsc::unbounded_channel();
                let _ = tx.send((chunk, reply));
                drop(tx);
                upload(access, rx).await;
            });
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(upload(access, rx));
        state(s).writes.insert(key, tx);
    }
    let st = state(s);
    if let Some(pieces) = st.writes.get(&key) {
        let _ = pieces.send((chunk, reply));
    }
    if last {
        st.writes.remove(&key);
    }
}

/// One upload: each chunk answered once written; once one failed, it and
/// every later one are answered with that failure, and the file stays as it
/// was (its temporary file removed before the failure is answered). Ends
/// after the last chunk, or when the caller's connection ends.
async fn upload(access: Access, mut pieces: mpsc::UnboundedReceiver<Piece>) {
    let mut upload: Result<Option<Upload>, String> = Ok(None);
    while let Some((chunk, reply)) = pieces.recv().await {
        let last = chunk.last;
        let result = match &mut upload {
            Err(failed) => Err(failed.clone()),
            Ok(open) => take(&access, open, chunk).await,
        };
        if let Err(failed) = &result
            && let Ok(Some(open)) = std::mem::replace(&mut upload, Err(failed.clone()))
        {
            open.abandon().await;
        }
        let _ = reply.send(result.map_err(refused));
        if last {
            return;
        }
    }
    if let Ok(Some(open)) = upload {
        open.abandon().await;
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
    let (uploads, watches) = (st.writes.len(), st.watches.len());
    st.writes.retain(|&(owner, _), _| owner != conn);
    st.watches.retain(|_, (owner, _)| *owner != conn);
    let (uploads, watches) = (uploads - st.writes.len(), watches - st.watches.len());
    tracing::info!(
        conn,
        uploads,
        watches,
        "fs: the connection ended, its uploads and watches went"
    );
}

/// How many watches `conn` has.
fn watches_of(st: &FsState, conn: u64) -> usize {
    st.watches
        .values()
        .filter(|(owner, _)| *owner == conn)
        .count()
}

fn too_many_watches() -> BusError {
    limit(format!(
        "{MAX_WATCHES_PER_CONNECTION} watches of this connection already"
    ))
}

/// `fs.watch`: answered once the server watches. A connection that ended,
/// or has [`MAX_WATCHES_PER_CONNECTION`] watches, gets none.
fn watch(
    core: Core,
    conn: u64,
    request: WatchRequest,
) -> impl Future<Output = Result<WatchReply, BusError>> + Send + 'static {
    let begun = queue(&core.supervisor, move |s| {
        let access = s
            .files
            .as_ref()
            .map(files::Files::access)
            .ok_or_else(|| BusError::unavailable("files are turned off on this server"))?;
        if watches_of(state(s), conn) >= MAX_WATCHES_PER_CONNECTION {
            return Err(too_many_watches());
        }
        if !on_connection_end(s, "fs", conn, forget_connection) {
            return Err(BusError::unavailable("the connection ended"));
        }
        let st = state(s);
        st.next_watch += 1;
        Ok((access, st.next_watch))
    });
    async move {
        let (access, watch_id) = asked(begun?).await??;
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
            // Unscoped: the broker's API has no scope of one connection, so
            // any subscriber with the right to `fs.changed.*` gets it.
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
            // A connection that ended meanwhile gets no watch; the limit is
            // checked again, for watches set up side by side.
            let alive = s
                .bus
                .broker()
                .is_some_and(|broker| broker.principal_of(conn).is_some());
            if !alive {
                return Err(BusError::unavailable("the connection ended"));
            }
            let st = state(s);
            if watches_of(st, conn) >= MAX_WATCHES_PER_CONNECTION {
                return Err(too_many_watches());
            }
            st.watches.insert(watch_id, (conn, handle));
            Ok(())
        });
        asked(kept?).await??;
        Ok(WatchReply { watch_id })
    }
}
