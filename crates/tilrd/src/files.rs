//! The files protocol: lets a client look at and change the files of the
//! server's machine, for a file browser and an editor (`FsList`, `FsStat`,
//! `FsRead`, `FsWrite`, `FsMkdir`, `FsRename`, `FsDelete`, `FsWatch`,
//! `FsUnwatch`; see tilr.proto; on the bus `fs.*`, `bus/core/fs.rs`).
//!
//! Only the directories in `[files] roots` of the configuration file are
//! served, the home directory by default; a path that leads out of them, by
//! `..` or through a symbolic link, is refused. With `readonly` nothing can be
//! changed. With `enabled = false` or `--no-files` the feature is not offered
//! at all.
//!
//! What touches the disk is [`Access`], which knows no protocol: paths in,
//! [`Entry`]s, chunks, [`Change`]s and messages for the user out. [`Files`]
//! is the `tilr.v1` side on top of it. The supervisor's actor only routes:
//! every look at the disk happens in a task of its own (`tokio::fs`,
//! `spawn_blocking`), which hands its answer back through the [`Sink`] as an
//! [`Output`].

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use bytes::Bytes;
use notify::Watcher;
use tilr_proto::{
    FileKind, FsAck, FsChange, FsChanged, FsData, FsEntries, FsEntry, FsInfo, FsRead, FsWrite,
    client_message::Msg, server_message,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::clipboard::human_size;
use crate::outbox::ToClient;

/// Bytes in one `FsData` chunk.
pub const CHUNK: usize = 256 * 1024;
/// How long changes to a watched directory are gathered before they are
/// reported, so that a burst to one entry comes as one.
const DEBOUNCE: Duration = Duration::from_millis(100);
/// Part of the name of the temporary file a write goes to before it is
/// renamed over the file. A watch does not report these.
const TEMP_MARK: &str = ".tilr-write-";
/// Directory of the runtime directory where each write in progress notes
/// its temporary file (see [`note_write`]).
const WRITES_DIR: &str = "writes";

/// What the configuration allows; see config.rs.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// As written: absolute paths, `~` for the home directory.
    pub roots: Vec<String>,
    pub readonly: bool,
    /// Largest content one read returns or one write takes.
    pub max_bytes: u64,
}

/// What the work done off the actor hands back to it.
pub enum Output {
    /// A message for a client.
    Send(u64, server_message::Msg),
    /// The watch a client asked for is set up; the supervisor keeps it for
    /// the client (see [`Files::add_watch`]) and acknowledges it.
    Watching {
        client: u64,
        request_id: u64,
        /// As the request named it.
        path: OsString,
        watch: Watch,
    },
}

/// Where answers go: to the supervisor's actor.
pub type Sink = Arc<dyn Fn(Output) + Send + Sync>;

/// What an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Link,
    /// A socket, a pipe, a device; or, as a link's `target`, nothing: the link
    /// is broken.
    Other,
}

/// One entry of a directory, not followed if it is a symbolic link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The last component of the path.
    pub name: OsString,
    pub kind: Kind,
    /// In bytes; a link's own.
    pub size: u64,
    /// Last modification, Unix time in milliseconds.
    pub mtime_ms: u64,
    /// Permission bits.
    pub mode: u32,
    /// What it is, links followed.
    pub target: Kind,
}

/// What happened to an entry of a watched directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Create,
    Modify,
    Remove,
}

/// A change a watch reports.
#[derive(Clone, Debug)]
pub struct Change {
    /// The directory, as the request named it.
    pub watch: String,
    /// The entry that changed, absolute and without links, or the directory.
    pub path: OsString,
    pub kind: ChangeKind,
}

/// Where a watch reports.
pub type ChangeSink = Arc<dyn Fn(Change) + Send + Sync>;

/// The roots and limits, shared with the tasks.
struct Shared {
    /// Canonical: absolute, without links.
    roots: Vec<PathBuf>,
    home: Option<PathBuf>,
    readonly: bool,
    max_bytes: u64,
}

/// The way to the shared files, without a protocol: what both `tilr.v1`'s
/// [`Files`] and the bus's `fs.*` call. The plain methods read the disk:
/// call them where that may block (`spawn_blocking`).
#[derive(Clone)]
pub struct Access(Arc<Shared>);

pub struct Files {
    access: Access,
    sink: Sink,
    /// Writes in progress, by client and transfer: where their chunks go.
    /// A refused write has a sender nobody receives from, so the rest of its
    /// chunks are dropped; the entry goes with the last chunk.
    writes: HashMap<(u64, u64), mpsc::UnboundedSender<FsWrite>>,
    /// The directories each client watches, named as in its `FsWatch`.
    watches: HashMap<u64, HashMap<OsString, Watch>>,
}

/// A directory being watched for one client. Dropping it stops the watch.
pub struct Watch {
    watcher: Option<notify::RecommendedWatcher>,
    task: JoinHandle<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
        // Stopping a watcher can wait on its thread; not on the actor.
        if let Some(watcher) = self.watcher.take() {
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => drop(runtime.spawn_blocking(move || drop(watcher))),
                Err(_) => drop(watcher),
            }
        }
    }
}

impl Files {
    /// Look up the roots. One that does not exist is left out, with a
    /// warning. Reads the disk: call at startup, not from the actor.
    pub fn new(settings: Settings, sink: Sink) -> Self {
        let home = std::env::home_dir().and_then(|home| fs::canonicalize(home).ok());
        let mut roots = Vec::new();
        for root in &settings.roots {
            let found = expand(Named(OsStr::new(root)), home.as_deref())
                .and_then(|path| fs::canonicalize(&path).map_err(|err| describe(&err)));
            match found {
                Ok(root) => roots.push(root),
                Err(err) => tracing::warn!(root, "files: root left out: {err}"),
            }
        }
        tracing::info!(
            roots = ?roots,
            readonly = settings.readonly,
            max = %human_size(settings.max_bytes),
            "files on"
        );
        Self {
            access: Access(Arc::new(Shared {
                roots,
                home,
                readonly: settings.readonly,
                max_bytes: settings.max_bytes,
            })),
            sink,
            writes: HashMap::new(),
            watches: HashMap::new(),
        }
    }

    pub fn max_bytes(&self) -> u64 {
        self.access.max_bytes()
    }

    /// The way to the files, for the bus.
    pub fn access(&self) -> Access {
        self.access.clone()
    }

    /// Take a request from `client`, whose queue is `to_client`: file
    /// content goes there directly, by its bulk lane. Messages that are not
    /// about files are ignored.
    pub fn on_message(&mut self, client: u64, to_client: &ToClient, msg: Msg) {
        let access = self.access.clone();
        let sink = Arc::clone(&self.sink);
        let reply = move |msg: server_message::Msg| sink(Output::Send(client, msg));
        let ack = |request_id: u64, error: String| {
            server_message::Msg::FsAck(FsAck { request_id, error })
        };
        match msg {
            Msg::FsList(list) => blocking(move || {
                let path = requested(&list.path, &list.path_bytes);
                let answer = match access.list(&path) {
                    Ok((path, entries)) => {
                        let (path, path_bytes) = name_of(path.as_os_str());
                        FsEntries {
                            request_id: list.request_id,
                            path,
                            entries: entries.into_iter().map(FsEntry::from).collect(),
                            error: String::new(),
                            path_bytes,
                        }
                    }
                    Err(error) => FsEntries {
                        request_id: list.request_id,
                        path: list.path,
                        entries: Vec::new(),
                        error,
                        path_bytes: list.path_bytes,
                    },
                };
                reply(server_message::Msg::FsEntries(answer));
            }),
            Msg::FsStat(stat) => blocking(move || {
                let path = requested(&stat.path, &stat.path_bytes);
                let answer = match access.stat(&path) {
                    Ok((path, entry)) => {
                        let (path, path_bytes) = name_of(path.as_os_str());
                        FsInfo {
                            request_id: stat.request_id,
                            path,
                            entry: Some(entry.into()),
                            error: String::new(),
                            path_bytes,
                        }
                    }
                    Err(error) => FsInfo {
                        request_id: stat.request_id,
                        path: stat.path,
                        entry: None,
                        error,
                        path_bytes: stat.path_bytes,
                    },
                };
                reply(server_message::Msg::FsInfo(answer));
            }),
            Msg::FsRead(read) => {
                let to_client = to_client.clone();
                tokio::spawn(async move {
                    let request_id = read.request_id;
                    match send_read(&access, read, &to_client).await {
                        Ok(()) | Err(Gone) => {}
                        Err(Failed(error)) => {
                            let _ = to_client
                                .send_bulk(encode(server_message::Msg::FsData(FsData {
                                    request_id,
                                    last: true,
                                    error,
                                    ..Default::default()
                                })))
                                .await;
                        }
                    }
                });
            }
            Msg::FsWrite(write) => self.on_write(client, write),
            Msg::FsMkdir(mkdir) => blocking(move || {
                let path = requested(&mkdir.path, &mkdir.path_bytes);
                let error = access.mkdir(&path);
                reply(ack(mkdir.request_id, error.err().unwrap_or_default()));
            }),
            Msg::FsRename(rename) => blocking(move || {
                let from = requested(&rename.from, &rename.from_bytes);
                let to = requested(&rename.to, &rename.to_bytes);
                let error = access.rename(&from, &to);
                reply(ack(rename.request_id, error.err().unwrap_or_default()));
            }),
            Msg::FsDelete(delete) => blocking(move || {
                let path = requested(&delete.path, &delete.path_bytes);
                let error = access.delete(&path, delete.recursive);
                reply(ack(delete.request_id, error.err().unwrap_or_default()));
            }),
            Msg::FsWatch(watch) => {
                let sink = Arc::clone(&self.sink);
                blocking(move || {
                    let path = requested(&watch.path, &watch.path_bytes);
                    let changes: ChangeSink = {
                        let sink = Arc::clone(&sink);
                        Arc::new(move |change: Change| {
                            let (path, path_bytes) = name_of(&change.path);
                            sink(Output::Send(
                                client,
                                server_message::Msg::FsChanged(FsChanged {
                                    request_id: 0,
                                    watch: change.watch.clone(),
                                    path,
                                    kind: FsChange::from(change.kind).into(),
                                    path_bytes,
                                }),
                            ));
                            #[cfg(feature = "failpoints")]
                            tracing::info!(
                                client,
                                watch = %change.watch,
                                kind = ?change.kind,
                                "files change queued"
                            );
                        })
                    };
                    match access.watch(&path, &watch.path, changes) {
                        Ok(handle) => sink(Output::Watching {
                            client,
                            request_id: watch.request_id,
                            path,
                            watch: handle,
                        }),
                        Err(error) => reply(ack(watch.request_id, error)),
                    }
                });
            }
            Msg::FsUnwatch(unwatch) => {
                let path = requested(&unwatch.path, &unwatch.path_bytes);
                let removed = self
                    .watches
                    .get_mut(&client)
                    .and_then(|watches| watches.remove(&path));
                let error = match removed {
                    Some(_) => String::new(),
                    None => format!("{}: not watched", Named(&path)),
                };
                reply(ack(unwatch.request_id, error));
            }
            _ => {}
        }
    }

    /// Keep a watch that is set up, replacing the one the client had on the
    /// same path. Returns the acknowledgement to send.
    pub fn add_watch(
        &mut self,
        client: u64,
        request_id: u64,
        path: OsString,
        watch: Watch,
    ) -> server_message::Msg {
        self.watches.entry(client).or_default().insert(path, watch);
        server_message::Msg::FsAck(FsAck {
            request_id,
            error: String::new(),
        })
    }

    /// Forget a client that is gone: its writes in progress are abandoned
    /// (the files stay as they were) and its watches stop.
    pub fn drop_client(&mut self, client: u64) {
        self.writes.retain(|&(owner, _), _| owner != client);
        self.watches.remove(&client);
    }

    fn on_write(&mut self, client: u64, write: FsWrite) {
        let key = (client, write.transfer_id);
        let last = write.last;
        match self.writes.get(&key) {
            // The receiving task may have given up already; then the chunk
            // is dropped.
            Some(chunks) => {
                let _ = chunks.send(write);
            }
            None => {
                let (chunks, incoming) = mpsc::unbounded_channel();
                if self.access.readonly() {
                    (self.sink)(Output::Send(
                        client,
                        server_message::Msg::FsAck(FsAck {
                            request_id: write.request_id,
                            error: READONLY.into(),
                        }),
                    ));
                    drop(incoming);
                } else {
                    let _ = chunks.send(write);
                    let access = self.access.clone();
                    let sink = Arc::clone(&self.sink);
                    tokio::spawn(async move {
                        if let Some((request_id, error)) = receive(access, incoming).await {
                            sink(Output::Send(
                                client,
                                server_message::Msg::FsAck(FsAck { request_id, error }),
                            ));
                        }
                    });
                }
                self.writes.insert(key, chunks);
            }
        }
        if last {
            self.writes.remove(&key);
        }
    }
}

const READONLY: &str = "files are read-only on this server";

/// How a read ends early.
enum ReadError {
    /// What to tell the client.
    Failed(String),
    /// The client's connection is gone: nobody to tell.
    Gone,
}
use ReadError::{Failed, Gone};

impl From<String> for ReadError {
    fn from(error: String) -> Self {
        Failed(error)
    }
}

fn encode(msg: server_message::Msg) -> bytes::Bytes {
    tilr_proto::encode(&tilr_proto::ServerMessage::from(msg))
}

fn blocking(work: impl FnOnce() + Send + 'static) {
    drop(tokio::task::spawn_blocking(work));
}

/// Send the part of a file `read` asks for as `FsData` chunks, each waiting
/// for room in the client's bulk lane: a client that takes them slowly is
/// read for slowly.
async fn send_read(access: &Access, read: FsRead, to_client: &ToClient) -> Result<(), ReadError> {
    let path = requested(&read.path, &read.path_bytes);
    let mut reader = access.open_read(path, read.offset, read.length).await?;
    while let Some(data) = reader.next().await? {
        let last = reader.done();
        let chunk = encode(server_message::Msg::FsData(FsData {
            request_id: read.request_id,
            total_size: reader.total,
            data,
            last,
            error: String::new(),
            file_size: reader.file_size,
        }));
        to_client.send_bulk(chunk).await.map_err(|_| Gone)?;
    }
    Ok(())
}

/// Take the chunks of one write as they come into an [`Upload`]. Returns the
/// acknowledgement to send: `None` when the client left before the end.
async fn receive(
    access: Access,
    mut chunks: mpsc::UnboundedReceiver<FsWrite>,
) -> Option<(u64, String)> {
    let first = chunks.recv().await?;
    let request_id = first.request_id;
    let start = WriteStart {
        path: requested(&first.path, &first.path_bytes),
        total_size: first.total_size,
        create: first.create,
        truncate: first.truncate,
    };
    let mut upload = match access.begin_write(start).await {
        Ok(upload) => upload,
        Err(error) => return Some((request_id, error)),
    };
    let mut chunk = first;
    loop {
        if let Err(error) = upload.take(&chunk.data).await {
            // Its temporary file is gone before the failure is told.
            upload.abandon().await;
            return Some((request_id, error));
        }
        if chunk.last {
            break;
        }
        // The client left in the middle: nobody to answer.
        let Some(next) = chunks.recv().await else {
            upload.abandon().await;
            return None;
        };
        chunk = next;
    }
    Some((request_id, upload.finish().await.err().unwrap_or_default()))
}

/// The first chunk of a write: what to replace.
#[derive(Clone, Debug)]
pub struct WriteStart {
    pub path: OsString,
    /// Size of the whole content.
    pub total_size: u64,
    /// The file may be missing and is created.
    pub create: bool,
    /// The file may exist and its content is replaced.
    pub truncate: bool,
}

/// A read in progress ([`Access::open_read`]): its chunks of at most
/// [`CHUNK`] bytes, in order.
pub struct Reader {
    /// Bytes the whole read carries.
    pub total: u64,
    /// Size of the file.
    pub file_size: u64,
    path: OsString,
    file: tokio::fs::File,
    sent: u64,
    done: bool,
}

impl Reader {
    /// The next chunk; `None` after the last. A read of nothing has one empty
    /// chunk.
    pub async fn next(&mut self) -> Result<Option<Bytes>, String> {
        if self.done {
            return Ok(None);
        }
        let path = Named(&self.path);
        let want = (self.total - self.sent).min(CHUNK as u64) as usize;
        let mut data = vec![0; want];
        let mut filled = 0;
        while filled < want {
            match self.file.read(&mut data[filled..]).await {
                Ok(0) => return Err(format!("{path}: the file got shorter while it was read")),
                Ok(n) => filled += n,
                Err(err) => return Err(failed(path, &err)),
            }
        }
        self.sent += want as u64;
        self.done = self.sent == self.total;
        Ok(Some(data.into()))
    }

    /// Whether the chunk [`next`](Self::next) gave last was the last.
    pub fn done(&self) -> bool {
        self.done
    }
}

/// A write in progress ([`Access::begin_write`]): its content goes to a
/// temporary file next to the file, renamed over it by [`finish`](Self::finish).
/// Dropped before, the temporary file is removed and the file stays as it was.
pub struct Upload {
    path: OsString,
    target: PathBuf,
    temp: PathBuf,
    note: Option<PathBuf>,
    file: Option<tokio::fs::File>,
    total: u64,
    written: u64,
    committed: bool,
}

impl Upload {
    /// Append a chunk.
    pub async fn take(&mut self, data: &[u8]) -> Result<(), String> {
        let path = Named(&self.path);
        self.written += data.len() as u64;
        if self.written > self.total {
            return Err(format!(
                "{path}: the write carries more data than it announced"
            ));
        }
        let Some(file) = &mut self.file else {
            return Err(format!("{path}: the write is over"));
        };
        file.write_all(data).await.map_err(|err| failed(path, &err))
    }

    /// Check the content is whole, put it on disk and rename it over the file.
    /// The temporary file (if the write failed) and the note of the write
    /// are gone before this returns.
    pub async fn finish(mut self) -> Result<(), String> {
        let outcome = self.commit().await;
        self.clean().await;
        outcome
    }

    /// Give the write up: its temporary file and its note are removed before
    /// this returns, and the file stays as it was.
    pub async fn abandon(mut self) {
        self.clean().await;
    }

    /// Remove what the write leaves behind now, off the runtime's threads.
    async fn clean(&mut self) {
        if let Some(clean) = self.leftovers() {
            let _ = tokio::task::spawn_blocking(clean).await;
        }
    }

    /// What removes the temporary file (unless it was renamed over the file)
    /// and the note; `None` if nothing is left. Taken once.
    fn leftovers(&mut self) -> Option<impl FnOnce() + Send + 'static> {
        let temp = (!std::mem::replace(&mut self.committed, true)).then(|| self.temp.clone());
        let note = self.note.take();
        if temp.is_none() && note.is_none() {
            return None;
        }
        let file = self.file.take();
        Some(move || {
            drop(file);
            if let Some(temp) = temp {
                let _ = fs::remove_file(temp);
            }
            if let Some(note) = note {
                let _ = fs::remove_file(note);
            }
        })
    }

    async fn commit(&mut self) -> Result<(), String> {
        let path = Named(&self.path);
        let (written, total) = (self.written, self.total);
        if written != total {
            return Err(format!(
                "{path}: the write ended after {written} of {total} bytes"
            ));
        }
        let Some(mut file) = self.file.take() else {
            return Err(format!("{path}: the write is over"));
        };
        file.flush().await.map_err(|err| failed(path, &err))?;
        // On disk before the answer: the data before the rename, the new name
        // (the directory) after it.
        file.sync_all().await.map_err(|err| failed(path, &err))?;
        drop(file);
        let (from, to) = (self.temp.clone(), self.target.clone());
        tokio::task::spawn_blocking(move || crate::durable::rename_synced(&from, &to))
            .await
            .map_err(|err| err.to_string())?
            .map_err(|err| failed(Named(&self.path), &err))?;
        self.committed = true;
        // Hold only the writer: the independent watcher must remain able to
        // report the committed file before this write is acknowledged.
        #[cfg(feature = "failpoints")]
        tokio::task::spawn_blocking(|| crate::fail::point("files-write-before-ack"))
            .await
            .map_err(|err| err.to_string())?
            .map_err(|err| err.to_string())?;
        Ok(())
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        let Some(clean) = self.leftovers() else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(clean)),
            Err(_) => clean(),
        }
    }
}

impl Access {
    pub fn max_bytes(&self) -> u64 {
        self.0.max_bytes
    }

    pub fn readonly(&self) -> bool {
        self.0.readonly
    }

    /// The entries of a directory, sorted by name, and the directory as an
    /// absolute path without links.
    pub fn list(&self, path: &OsStr) -> Result<(PathBuf, Vec<Entry>), String> {
        self.0.list(Named(path))
    }

    /// One entry, not followed if it is a link, and its path.
    pub fn stat(&self, path: &OsStr) -> Result<(PathBuf, Entry), String> {
        self.0.stat(Named(path))
    }

    /// Make a directory; its parent must exist.
    pub fn mkdir(&self, path: &OsStr) -> Result<(), String> {
        self.writable()?;
        let named = Named(path);
        let entry = self.0.entry(named)?;
        fs::create_dir(&entry).map_err(|err| failed(named, &err))
    }

    /// Move or rename an entry; `to` must not exist.
    pub fn rename(&self, from: &OsStr, to: &OsStr) -> Result<(), String> {
        self.writable()?;
        self.0.rename(Named(from), Named(to))
    }

    /// Remove a file, a link or a directory (`recursive`: with what is in it).
    pub fn delete(&self, path: &OsStr, recursive: bool) -> Result<(), String> {
        self.writable()?;
        self.0.delete(Named(path), recursive)
    }

    /// Start watching a directory; `named` is the path as the request's
    /// string has it, for [`Change::watch`]. Dropping the [`Watch`] stops it.
    /// Must run where it may block, inside the runtime.
    pub fn watch(&self, path: &OsStr, named: &str, sink: ChangeSink) -> Result<Watch, String> {
        self.0.watch(Named(path), named, sink)
    }

    /// Open the part of a file from `offset`, `length` bytes (0: to the end),
    /// at most [`Access::max_bytes`].
    pub async fn open_read(
        &self,
        path: OsString,
        offset: u64,
        length: u64,
    ) -> Result<Reader, String> {
        let real = {
            let shared = Arc::clone(&self.0);
            let path = path.clone();
            tokio::task::spawn_blocking(move || shared.follow(Named(&path)))
                .await
                .map_err(|err| err.to_string())??
        };
        let named = Named(&path);
        let mut file = tokio::fs::File::open(&real)
            .await
            .map_err(|err| failed(named, &err))?;
        let meta = file.metadata().await.map_err(|err| failed(named, &err))?;
        if meta.is_dir() {
            return Err(format!("{named}: is a directory"));
        }
        let file_size = meta.len();
        let available = file_size.saturating_sub(offset);
        let total = match length {
            0 => available,
            length => length.min(available),
        };
        if total > self.0.max_bytes {
            return Err(format!(
                "{named}: too large to read at once: {}, the server sends at most {}",
                human_size(total),
                human_size(self.0.max_bytes)
            ));
        }
        if offset > 0 {
            file.seek(io::SeekFrom::Start(offset))
                .await
                .map_err(|err| failed(named, &err))?;
        }
        Ok(Reader {
            total,
            file_size,
            path,
            file,
            sent: 0,
            done: false,
        })
    }

    /// Check the first chunk of a write and open its temporary file.
    pub async fn begin_write(&self, start: WriteStart) -> Result<Upload, String> {
        self.writable()?;
        let prepared = {
            let shared = Arc::clone(&self.0);
            let start = start.clone();
            tokio::task::spawn_blocking(move || shared.prepare_write(&start))
                .await
                .map_err(|err| err.to_string())??
        };
        let (target, temp, mode) = prepared;
        let note = {
            let temp = temp.clone();
            tokio::task::spawn_blocking(move || note_write(&writes_dir(), &temp))
                .await
                .ok()
                .flatten()
        };
        let mut upload = Upload {
            path: start.path,
            target,
            temp,
            note,
            file: None,
            total: start.total_size,
            written: 0,
            committed: false,
        };
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let file = match options.open(&upload.temp).await {
            Ok(file) => file,
            Err(err) => {
                // Not ours: nothing to remove but the note.
                upload.committed = true;
                let error = failed(Named(&upload.path), &err);
                upload.abandon().await;
                return Err(error);
            }
        };
        if let Some(mode) = mode
            && let Err(err) = file.set_permissions(fs::Permissions::from_mode(mode)).await
        {
            let error = failed(Named(&upload.path), &err);
            upload.file = Some(file);
            upload.abandon().await;
            return Err(error);
        }
        upload.file = Some(file);
        Ok(upload)
    }

    fn writable(&self) -> Result<(), String> {
        if self.0.readonly {
            Err(READONLY.into())
        } else {
            Ok(())
        }
    }
}

impl Shared {
    /// The path a client named, with `~` expanded: absolute, but not yet
    /// checked against the roots.
    fn expand(&self, path: Named) -> Result<PathBuf, String> {
        expand(path, self.home.as_deref())
    }

    fn inside(&self, path: &Path, strictly: bool) -> bool {
        self.roots
            .iter()
            .any(|root| path.starts_with(root) && (!strictly || path != root))
    }

    fn outside(path: Named) -> String {
        format!("{path}: outside the directories this server shares")
    }

    /// What `path` leads to, links followed: inside a root, or a root
    /// itself.
    fn follow(&self, path: Named) -> Result<PathBuf, String> {
        let expanded = self.expand(path)?;
        match fs::canonicalize(&expanded) {
            Ok(real) if self.inside(&real, false) => Ok(real),
            Ok(_) => Err(Self::outside(path)),
            // Do not tell what is or is not there outside the roots.
            Err(_) if !self.lexically_inside(&expanded) => Err(Self::outside(path)),
            Err(err) => Err(failed(path, &err)),
        }
    }

    /// The entry `path` names itself, a link not followed: the directory it
    /// is in without links, and its name. It need not exist, but must be in a
    /// root, not a root itself.
    fn entry(&self, path: Named) -> Result<PathBuf, String> {
        let expanded = normalize(&self.expand(path)?);
        let (Some(parent), Some(name)) = (expanded.parent(), expanded.file_name()) else {
            return Err(Self::outside(path));
        };
        let parent = match fs::canonicalize(parent) {
            Ok(parent) => parent,
            Err(_) if !self.lexically_inside(&expanded) => return Err(Self::outside(path)),
            Err(err) => return Err(failed(path, &err)),
        };
        let entry = parent.join(name);
        if self.inside(&entry, true) {
            Ok(entry)
        } else {
            Err(Self::outside(path))
        }
    }

    /// Whether `path`, taken as written, is in a root: for telling "outside"
    /// from "missing" when the disk cannot say.
    fn lexically_inside(&self, path: &Path) -> bool {
        let path = normalize(path);
        self.inside(&path, false)
    }

    fn list(&self, path: Named) -> Result<(PathBuf, Vec<Entry>), String> {
        let dir = self.follow(path)?;
        let mut entries = Vec::new();
        for item in fs::read_dir(&dir).map_err(|err| failed(path, &err))? {
            let item = item.map_err(|err| failed(path, &err))?;
            let Ok(meta) = item.path().symlink_metadata() else {
                // Gone since the directory was read.
                continue;
            };
            entries.push(entry_of(item.file_name(), &item.path(), &meta));
        }
        // As `tilr.v1` sorted its entries: by the name, then its bytes.
        entries.sort_by_cached_key(|entry| name_of(&entry.name));
        Ok((dir, entries))
    }

    fn stat(&self, path: Named) -> Result<(PathBuf, Entry), String> {
        // A root is not an entry of anything shared, but can be looked at.
        let (real, meta) = match self.entry(path) {
            Ok(entry) => {
                let meta = entry.symlink_metadata().map_err(|err| failed(path, &err))?;
                (entry, meta)
            }
            Err(not_entry) => {
                let real = self.follow(path).map_err(|_| not_entry)?;
                let meta = real.metadata().map_err(|err| failed(path, &err))?;
                (real, meta)
            }
        };
        let name = real
            .file_name()
            .map_or_else(|| OsString::from("/"), ToOwned::to_owned);
        let entry = entry_of(name, &real, &meta);
        Ok((real, entry))
    }

    /// Check the first chunk of a write: the file to replace, the temporary
    /// file next to it, and the permissions to give it (those of the file it
    /// replaces).
    fn prepare_write(&self, first: &WriteStart) -> Result<(PathBuf, PathBuf, Option<u32>), String> {
        let path = Named(&first.path);
        if first.total_size > self.max_bytes {
            return Err(format!(
                "{path}: too large: {}, the server takes at most {}",
                human_size(first.total_size),
                human_size(self.max_bytes)
            ));
        }
        let entry = self.entry(path)?;
        let (target, existing) = match entry.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                // Written through: the file it points to is replaced.
                let real = self.follow(path)?;
                if !self.inside(&real, true) {
                    return Err(Self::outside(path));
                }
                let meta = real.metadata().map_err(|err| failed(path, &err))?;
                (real, Some(meta))
            }
            Ok(meta) => (entry, Some(meta)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (entry, None),
            Err(err) => return Err(failed(path, &err)),
        };
        let mode = match &existing {
            Some(meta) if meta.is_dir() => return Err(format!("{path}: is a directory")),
            Some(meta) if !meta.is_file() => return Err(format!("{path}: not a regular file")),
            Some(_) if !first.truncate => return Err(format!("{path}: already exists")),
            Some(meta) => Some(meta.mode() & 0o7777),
            None if !first.create => return Err(format!("{path}: no such file")),
            None => None,
        };
        let name = target.file_name().unwrap_or_default().to_string_lossy();
        let temp = target.with_file_name(format!(
            ".{name}{TEMP_MARK}{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        ));
        Ok((target, temp, mode))
    }

    fn rename(&self, from: Named, to: Named) -> Result<(), String> {
        let source = self.entry(from)?;
        let target = self.entry(to)?;
        source
            .symlink_metadata()
            .map_err(|err| failed(from, &err))?;
        if target.symlink_metadata().is_ok() {
            return Err(format!("{to}: already exists"));
        }
        fs::rename(&source, &target).map_err(|err| failed(from, &err))
    }

    fn delete(&self, path: Named, recursive: bool) -> Result<(), String> {
        let entry = self.entry(path)?;
        let meta = entry.symlink_metadata().map_err(|err| failed(path, &err))?;
        let removed = if meta.is_dir() && recursive {
            fs::remove_dir_all(&entry)
        } else if meta.is_dir() {
            fs::remove_dir(&entry)
        } else {
            fs::remove_file(&entry)
        };
        removed.map_err(|err| failed(path, &err))
    }

    /// Start watching a directory. Must run where it may block, inside the
    /// runtime.
    /// `named` is the path as the request's string has it, for `Change::watch`.
    fn watch(&self, path: Named, named: &str, sink: ChangeSink) -> Result<Watch, String> {
        let dir = self.follow(path)?;
        let known: HashSet<PathBuf> = fs::read_dir(&dir)
            .map_err(|err| failed(path, &err))?
            .filter_map(|item| Some(item.ok()?.path()))
            .collect();
        let (events, incoming) = mpsc::unbounded_channel();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event
                    && !matches!(event.kind, notify::EventKind::Access(_))
                {
                    let _ = events.send(event.paths);
                }
            })
            .map_err(|err| format!("{path}: cannot watch: {err}"))?;
        watcher
            .watch(&dir, notify::RecursiveMode::NonRecursive)
            .map_err(|err| format!("{path}: cannot watch: {err}"))?;
        let task = tokio::spawn(report_changes(named.to_owned(), dir, known, incoming, sink));
        Ok(Watch {
            watcher: Some(watcher),
            task,
        })
    }
}

impl From<Kind> for FileKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::File => Self::File,
            Kind::Dir => Self::Dir,
            Kind::Link => Self::Link,
            Kind::Other => Self::Other,
        }
    }
}

impl From<Entry> for FsEntry {
    fn from(entry: Entry) -> Self {
        let (name, name_bytes) = name_of(&entry.name);
        Self {
            name,
            name_bytes,
            kind: FileKind::from(entry.kind).into(),
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            mode: entry.mode,
            target: FileKind::from(entry.target).into(),
        }
    }
}

impl From<ChangeKind> for FsChange {
    fn from(kind: ChangeKind) -> Self {
        match kind {
            ChangeKind::Create => Self::Create,
            ChangeKind::Modify => Self::Modify,
            ChangeKind::Remove => Self::Remove,
        }
    }
}

/// Turn what the watcher saw into [`Change`]s: each entry that changed once
/// per burst, a create or remove judged by whether it is there now and was
/// before.
async fn report_changes(
    watch: String,
    dir: PathBuf,
    mut known: HashSet<PathBuf>,
    mut incoming: mpsc::UnboundedReceiver<Vec<PathBuf>>,
    sink: ChangeSink,
) {
    while let Some(paths) = incoming.recv().await {
        let mut changed: Vec<PathBuf> = Vec::new();
        let add = |paths: Vec<PathBuf>, changed: &mut Vec<PathBuf>| {
            for path in paths {
                let ours = path == dir || path.parent() == Some(dir.as_path());
                let temp = path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(TEMP_MARK));
                if ours && !temp && !changed.contains(&path) {
                    changed.push(path);
                }
            }
        };
        add(paths, &mut changed);
        let until = Instant::now() + DEBOUNCE;
        while let Ok(Some(paths)) = tokio::time::timeout_at(until.into(), incoming.recv()).await {
            add(paths, &mut changed);
        }
        for path in changed {
            let exists = tokio::fs::symlink_metadata(&path).await.is_ok();
            let kind = if path == dir {
                if exists {
                    // The directory's own times: its entries say more.
                    continue;
                }
                ChangeKind::Remove
            } else if !exists {
                if !known.remove(&path) {
                    continue;
                }
                ChangeKind::Remove
            } else if known.insert(path.clone()) {
                ChangeKind::Create
            } else {
                ChangeKind::Modify
            };
            sink(Change {
                watch: watch.clone(),
                path: path.into_os_string(),
                kind,
            });
        }
    }
}

fn writes_dir() -> PathBuf {
    crate::clipboard::runtime_dir().join(WRITES_DIR)
}

/// Remove what writes cut short by the end of an earlier server, or of this
/// one before a reload, left (see [`sweep_writes`]). Call at startup.
pub fn sweep() {
    sweep_writes(&writes_dir());
}

/// Note a write in progress: a file in `dir` named after this process
/// (`<pid>-<random>`) holding the path of the write's temporary file. The
/// write removes it when it is done; one a process left (it was killed, or
/// exec'd itself in a reload, with the write's task in the middle) tells
/// [`sweep_writes`] which temporary file is nobody's. `None` if it cannot
/// be made: the write goes on without it.
fn note_write(dir: &Path, temp: &Path) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    crate::clipboard::private_dir(dir)
        .map_err(|err| tracing::debug!("files: no note of the write: {err:#}"))
        .ok()?;
    let note = dir.join(format!(
        "{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    match fs::write(&note, temp.as_os_str().as_bytes()) {
        Ok(()) => Some(note),
        Err(err) => {
            tracing::debug!(note = %note.display(), "files: no note of the write: {err}");
            let _ = fs::remove_file(&note);
            None
        }
    }
}

/// Remove the temporary files of writes that will never finish: those noted
/// (see [`note_write`]) by a process that is gone, or by this one before it
/// started over (an exec keeps the pid, and nothing is written yet). Runs at
/// startup, before any write. Only a file whose name has [`TEMP_MARK`] is
/// removed, whatever a note says.
fn sweep_writes(dir: &Path) {
    use std::os::unix::ffi::OsStrExt;

    let Ok(notes) = fs::read_dir(dir) else {
        return;
    };
    let me = std::process::id();
    for note in notes.flatten() {
        let name = note.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|name| name.split_once('-'))
            .and_then(|(pid, _)| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != me && alive(pid) {
            continue;
        }
        if let Ok(temp) = fs::read(note.path()) {
            let temp = PathBuf::from(std::ffi::OsStr::from_bytes(&temp));
            let ours = temp.file_name().is_some_and(|name| {
                name.as_bytes()
                    .windows(TEMP_MARK.len())
                    .any(|w| w == TEMP_MARK.as_bytes())
            });
            if ours && temp.is_absolute() {
                match fs::remove_file(&temp) {
                    Ok(()) => tracing::info!(
                        temp = %temp.display(),
                        "files: removed the temporary file of a write cut short"
                    ),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => tracing::warn!(temp = %temp.display(), "files: {err}"),
                }
            }
        }
        let _ = fs::remove_file(note.path());
    }
}

/// Whether a process with this pid exists.
fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the process exists.
    let found = unsafe { libc::kill(pid, 0) } == 0;
    found || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// `~` and `~/...` made the home directory; anything else must be absolute.
fn expand(path: Named, home: Option<&Path>) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStrExt;

    let home = || home.ok_or_else(|| format!("{path}: no home directory"));
    let bytes = path.0.as_bytes();
    if bytes == b"~" {
        return Ok(home()?.to_path_buf());
    }
    if let Some(rest) = bytes.strip_prefix(b"~/") {
        return Ok(home()?.join(OsStr::from_bytes(rest)));
    }
    if Path::new(path.0).is_absolute() {
        Ok(PathBuf::from(path.0))
    } else {
        Err(format!("{path:?}: not an absolute path"))
    }
}

/// A path as a request names it: the raw bytes when they are given (a name
/// that is not UTF-8), the string otherwise.
pub fn requested(path: &str, bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStrExt;

    if bytes.is_empty() {
        OsString::from(path)
    } else {
        OsStr::from_bytes(bytes).to_owned()
    }
}

/// A name or path for an answer: the string, and the raw bytes too if it is
/// not UTF-8 (the string then has U+FFFD for what is not).
pub fn name_of(name: &OsStr) -> (String, Vec<u8>) {
    use std::os::unix::ffi::OsStrExt;

    match name.to_str() {
        Some(name) => (name.to_owned(), Vec::new()),
        None => (
            name.to_string_lossy().into_owned(),
            name.as_bytes().to_vec(),
        ),
    }
}

/// A path a request named, shown in messages (as UTF-8, U+FFFD for what is
/// not).
#[derive(Clone, Copy)]
struct Named<'a>(&'a OsStr);

impl std::fmt::Display for Named<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.to_string_lossy())
    }
}

impl std::fmt::Debug for Named<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0.to_string_lossy(), f)
    }
}

/// `path` without `.` and with each `..` taking the component before it.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn kind_of(file_type: fs::FileType) -> Kind {
    if file_type.is_symlink() {
        Kind::Link
    } else if file_type.is_dir() {
        Kind::Dir
    } else if file_type.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

/// The entry at `path`, from its own metadata (a link's, not its target's).
fn entry_of(name: OsString, path: &Path, meta: &fs::Metadata) -> Entry {
    let kind = kind_of(meta.file_type());
    let target = match kind {
        Kind::Link => match path.metadata() {
            Ok(meta) if meta.is_dir() => Kind::Dir,
            Ok(meta) if meta.is_file() => Kind::File,
            _ => Kind::Other,
        },
        kind => kind,
    };
    Entry {
        name,
        kind,
        size: meta.len(),
        mtime_ms: meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_millis() as u64),
        mode: meta.mode() & 0o7777,
        target,
    }
}

/// What went wrong with `path`, for the user.
fn failed(path: Named, err: &io::Error) -> String {
    format!("{path}: {}", describe(err))
}

fn describe(err: &io::Error) -> String {
    use io::ErrorKind::*;
    match err.kind() {
        NotFound => "no such file or directory".into(),
        PermissionDenied => "permission denied".into(),
        AlreadyExists => "already exists".into(),
        DirectoryNotEmpty => "directory not empty".into(),
        NotADirectory => "not a directory".into(),
        IsADirectory => "is a directory".into(),
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_absolute_or_from_home() {
        let home = Path::new("/home/me");
        assert_eq!(expand(Named(OsStr::new("~")), Some(home)).unwrap(), home);
        assert_eq!(
            expand(Named(OsStr::new("~/a/b")), Some(home)).unwrap(),
            home.join("a/b")
        );
        assert_eq!(
            expand(Named(OsStr::new("/etc")), Some(home)).unwrap(),
            Path::new("/etc")
        );
        assert!(expand(Named(OsStr::new("a/b")), Some(home)).is_err());
        assert!(expand(Named(OsStr::new("~other")), Some(home)).is_err());
        assert!(expand(Named(OsStr::new("")), Some(home)).is_err());
        assert!(expand(Named(OsStr::new("~")), None).is_err());
    }

    #[test]
    fn the_temporary_file_of_a_write_whose_process_is_gone_is_swept() {
        let dir = std::env::temp_dir().join(format!(
            "tilr-files-sweep-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let notes = dir.join(WRITES_DIR);
        fs::create_dir_all(&notes).unwrap();
        let temp = |name: &str| dir.join(format!(".{name}{TEMP_MARK}0123456789ab"));
        // Noted by this process before it started over (a reload).
        fs::write(temp("mine"), b"half").unwrap();
        note_write(&notes, &temp("mine")).unwrap();
        // Noted by a process that is alive (pid 1): its write may go on.
        fs::write(temp("theirs"), b"half").unwrap();
        fs::write(
            notes.join("1-a"),
            temp("theirs").as_os_str().as_encoded_bytes(),
        )
        .unwrap();
        // A note naming something that is not a temporary file of a write.
        fs::write(dir.join("precious"), b"keep").unwrap();
        fs::write(
            notes.join(format!("{}-b", std::process::id())),
            dir.join("precious").as_os_str().as_encoded_bytes(),
        )
        .unwrap();

        sweep_writes(&notes);
        assert!(!temp("mine").exists(), "this process's leftover is removed");
        assert!(
            temp("theirs").exists(),
            "a live process's write is left alone"
        );
        assert!(
            dir.join("precious").exists(),
            "only temporary files are removed"
        );
        let left: Vec<_> = fs::read_dir(&notes)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["1-a"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dot_dot_takes_the_component_before_it() {
        assert_eq!(normalize(Path::new("/a/b/../c/./d")), Path::new("/a/c/d"));
        assert_eq!(normalize(Path::new("/a/../..")), Path::new("/"));
    }
}
