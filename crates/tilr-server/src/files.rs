//! The files protocol: lets a client look at and change the files of the
//! server's machine, for a file browser and an editor (`FsList`, `FsStat`,
//! `FsRead`, `FsWrite`, `FsMkdir`, `FsRename`, `FsDelete`, `FsWatch`,
//! `FsUnwatch`; see tilr.proto).
//!
//! Only the directories in `[files] roots` of the configuration file are
//! served, the home directory by default; a path that leads out of them, by
//! `..` or through a symbolic link, is refused. With `readonly` nothing can be
//! changed. With `enabled = false` or `--no-files` the feature is not offered
//! at all.
//!
//! The supervisor's actor only routes: every look at the disk happens in a
//! task of its own (`tokio::fs`, `spawn_blocking`), which hands its answer
//! back through the [`Sink`] as an [`Output`].

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use notify::Watcher;
use tilr_proto::{
    FileKind, FsAck, FsChange, FsChanged, FsData, FsEntries, FsEntry, FsInfo, FsRead, FsWrite,
    client_message::Msg, server_message,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::clipboard::human_size;

/// Bytes in one `FsData` chunk.
pub const CHUNK: usize = 256 * 1024;
/// How long changes to a watched directory are gathered before they are
/// reported, so that a burst to one entry comes as one.
const DEBOUNCE: Duration = Duration::from_millis(100);
/// Part of the name of the temporary file a write goes to before it is
/// renamed over the file. A watch does not report these.
const TEMP_MARK: &str = ".tilr-write-";

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
        path: String,
        watch: Watch,
    },
}

/// Where answers go: to the supervisor's actor.
pub type Sink = Arc<dyn Fn(Output) + Send + Sync>;

/// The roots and limits, shared with the tasks.
struct Shared {
    /// Canonical: absolute, without links.
    roots: Vec<PathBuf>,
    home: Option<PathBuf>,
    readonly: bool,
    max_bytes: u64,
}

pub struct Files {
    shared: Arc<Shared>,
    sink: Sink,
    /// Writes in progress, by client and transfer: where their chunks go.
    /// A refused write has a sender nobody receives from, so the rest of its
    /// chunks are dropped; the entry goes with the last chunk.
    writes: HashMap<(u64, u64), mpsc::UnboundedSender<FsWrite>>,
    /// The directories each client watches, named as in its `FsWatch`.
    watches: HashMap<u64, HashMap<String, Watch>>,
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
            let found = expand(root, home.as_deref())
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
            shared: Arc::new(Shared {
                roots,
                home,
                readonly: settings.readonly,
                max_bytes: settings.max_bytes,
            }),
            sink,
            writes: HashMap::new(),
            watches: HashMap::new(),
        }
    }

    pub fn max_bytes(&self) -> u64 {
        self.shared.max_bytes
    }

    /// Take a request. Messages that are not about files are ignored.
    pub fn on_message(&mut self, client: u64, msg: Msg) {
        let shared = Arc::clone(&self.shared);
        let sink = Arc::clone(&self.sink);
        let reply = move |msg: server_message::Msg| sink(Output::Send(client, msg));
        let ack = |request_id: u64, error: String| {
            server_message::Msg::FsAck(FsAck { request_id, error })
        };
        let changes = matches!(msg, Msg::FsMkdir(_) | Msg::FsRename(_) | Msg::FsDelete(_));
        if changes && shared.readonly {
            let request_id = match &msg {
                Msg::FsMkdir(m) => m.request_id,
                Msg::FsRename(m) => m.request_id,
                Msg::FsDelete(m) => m.request_id,
                _ => 0,
            };
            return reply(ack(request_id, READONLY.into()));
        }
        match msg {
            Msg::FsList(list) => blocking(move || {
                let answer = match shared.list(&list.path) {
                    Ok((path, entries)) => FsEntries {
                        request_id: list.request_id,
                        path: path.to_string_lossy().into_owned(),
                        entries,
                        error: String::new(),
                    },
                    Err(error) => FsEntries {
                        request_id: list.request_id,
                        path: list.path,
                        entries: Vec::new(),
                        error,
                    },
                };
                reply(server_message::Msg::FsEntries(answer));
            }),
            Msg::FsStat(stat) => blocking(move || {
                let answer = match shared.stat(&stat.path) {
                    Ok((path, entry)) => FsInfo {
                        request_id: stat.request_id,
                        path: path.to_string_lossy().into_owned(),
                        entry: Some(entry),
                        error: String::new(),
                    },
                    Err(error) => FsInfo {
                        request_id: stat.request_id,
                        path: stat.path,
                        entry: None,
                        error,
                    },
                };
                reply(server_message::Msg::FsInfo(answer));
            }),
            Msg::FsRead(read) => {
                tokio::spawn(async move {
                    let request_id = read.request_id;
                    if let Err(error) = shared.read(read, &reply).await {
                        reply(server_message::Msg::FsData(FsData {
                            request_id,
                            last: true,
                            error,
                            ..Default::default()
                        }));
                    }
                });
            }
            Msg::FsWrite(write) => self.on_write(client, write),
            Msg::FsMkdir(mkdir) => blocking(move || {
                let error = shared.entry(&mkdir.path).and_then(|path| {
                    fs::create_dir(&path).map_err(|err| failed(&mkdir.path, &err))
                });
                reply(ack(mkdir.request_id, error.err().unwrap_or_default()));
            }),
            Msg::FsRename(rename) => blocking(move || {
                let error = shared.rename(&rename.from, &rename.to);
                reply(ack(rename.request_id, error.err().unwrap_or_default()));
            }),
            Msg::FsDelete(delete) => blocking(move || {
                let error = shared.delete(&delete.path, delete.recursive);
                reply(ack(delete.request_id, error.err().unwrap_or_default()));
            }),
            Msg::FsWatch(watch) => {
                let sink = Arc::clone(&self.sink);
                blocking(
                    move || match shared.watch(&watch.path, Arc::clone(&sink), client) {
                        Ok(handle) => sink(Output::Watching {
                            client,
                            request_id: watch.request_id,
                            path: watch.path,
                            watch: handle,
                        }),
                        Err(error) => reply(ack(watch.request_id, error)),
                    },
                );
            }
            Msg::FsUnwatch(unwatch) => {
                let removed = self
                    .watches
                    .get_mut(&client)
                    .and_then(|watches| watches.remove(&unwatch.path));
                let error = match removed {
                    Some(_) => String::new(),
                    None => format!("{}: not watched", unwatch.path),
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
        path: String,
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
                if self.shared.readonly {
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
                    let shared = Arc::clone(&self.shared);
                    let sink = Arc::clone(&self.sink);
                    tokio::spawn(async move {
                        if let Some((request_id, error)) = shared.receive(incoming).await {
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

fn blocking(work: impl FnOnce() + Send + 'static) {
    drop(tokio::task::spawn_blocking(work));
}

impl Shared {
    /// The path a client named, with `~` expanded: absolute, but not yet
    /// checked against the roots.
    fn expand(&self, path: &str) -> Result<PathBuf, String> {
        expand(path, self.home.as_deref())
    }

    fn inside(&self, path: &Path, strictly: bool) -> bool {
        self.roots
            .iter()
            .any(|root| path.starts_with(root) && (!strictly || path != root))
    }

    fn outside(path: &str) -> String {
        format!("{path}: outside the directories this server shares")
    }

    /// What `path` leads to, links followed: inside a root, or a root
    /// itself.
    fn follow(&self, path: &str) -> Result<PathBuf, String> {
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
    fn entry(&self, path: &str) -> Result<PathBuf, String> {
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

    fn list(&self, path: &str) -> Result<(PathBuf, Vec<FsEntry>), String> {
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
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok((dir, entries))
    }

    fn stat(&self, path: &str) -> Result<(PathBuf, FsEntry), String> {
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

    async fn read(
        self: Arc<Self>,
        read: FsRead,
        reply: &(impl Fn(server_message::Msg) + Send + Sync),
    ) -> Result<(), String> {
        let path = read.path.clone();
        let real = {
            let shared = Arc::clone(&self);
            let path = path.clone();
            tokio::task::spawn_blocking(move || shared.follow(&path))
                .await
                .map_err(|err| err.to_string())??
        };
        let mut file = tokio::fs::File::open(&real)
            .await
            .map_err(|err| failed(&path, &err))?;
        let meta = file.metadata().await.map_err(|err| failed(&path, &err))?;
        if meta.is_dir() {
            return Err(format!("{path}: is a directory"));
        }
        let file_size = meta.len();
        let available = file_size.saturating_sub(read.offset);
        let total = match read.length {
            0 => available,
            length => length.min(available),
        };
        if total > self.max_bytes {
            return Err(format!(
                "{path}: too large to read at once: {}, the server sends at most {}",
                human_size(total),
                human_size(self.max_bytes)
            ));
        }
        if read.offset > 0 {
            file.seek(io::SeekFrom::Start(read.offset))
                .await
                .map_err(|err| failed(&path, &err))?;
        }
        let mut sent = 0u64;
        loop {
            let want = (total - sent).min(CHUNK as u64) as usize;
            let mut data = vec![0; want];
            let mut filled = 0;
            while filled < want {
                match file.read(&mut data[filled..]).await {
                    Ok(0) => return Err(format!("{path}: the file got shorter while it was read")),
                    Ok(n) => filled += n,
                    Err(err) => return Err(failed(&path, &err)),
                }
            }
            sent += want as u64;
            let last = sent == total;
            reply(server_message::Msg::FsData(FsData {
                request_id: read.request_id,
                total_size: total,
                data: data.into(),
                last,
                error: String::new(),
                file_size,
            }));
            if last {
                return Ok(());
            }
        }
    }

    /// Take the chunks of one write as they come and write them to a
    /// temporary file, renamed over the file after the last. Returns the
    /// acknowledgement to send: `None` when the client left before the end.
    async fn receive(
        self: Arc<Self>,
        mut chunks: mpsc::UnboundedReceiver<FsWrite>,
    ) -> Option<(u64, String)> {
        let first = chunks.recv().await?;
        let request_id = first.request_id;
        let shared = Arc::clone(&self);
        let check = {
            let first = FsWrite {
                data: Default::default(),
                ..first.clone()
            };
            tokio::task::spawn_blocking(move || shared.prepare_write(&first)).await
        };
        let (target, temp, mode) = match check {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => return Some((request_id, error)),
            Err(err) => return Some((request_id, err.to_string())),
        };
        let path = first.path.clone();
        let total = first.total_size;
        let outcome = async {
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            let mut file = options
                .open(&temp)
                .await
                .map_err(|err| failed(&path, &err))?;
            if let Some(mode) = mode {
                file.set_permissions(fs::Permissions::from_mode(mode))
                    .await
                    .map_err(|err| failed(&path, &err))?;
            }
            let mut written = 0u64;
            let mut chunk = first;
            loop {
                written += chunk.data.len() as u64;
                if written > total {
                    return Err(format!(
                        "{path}: the write carries more data than it announced"
                    ));
                }
                file.write_all(&chunk.data)
                    .await
                    .map_err(|err| failed(&path, &err))?;
                if chunk.last {
                    break;
                }
                // The client left in the middle: nobody to answer.
                chunk = chunks.recv().await.ok_or_else(String::new)?;
            }
            if written != total {
                return Err(format!(
                    "{path}: the write ended after {written} of {total} bytes"
                ));
            }
            file.flush().await.map_err(|err| failed(&path, &err))?;
            drop(file);
            tokio::fs::rename(&temp, &target)
                .await
                .map_err(|err| failed(&path, &err))
        }
        .await;
        match outcome {
            Ok(()) => Some((request_id, String::new())),
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp).await;
                (!error.is_empty()).then_some((request_id, error))
            }
        }
    }

    /// Check the first chunk of a write: the file to replace, the temporary
    /// file next to it, and the permissions to give it (those of the file it
    /// replaces).
    fn prepare_write(&self, first: &FsWrite) -> Result<(PathBuf, PathBuf, Option<u32>), String> {
        let path = &first.path;
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

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
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

    fn delete(&self, path: &str, recursive: bool) -> Result<(), String> {
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
    fn watch(&self, path: &str, sink: Sink, client: u64) -> Result<Watch, String> {
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
        let task = tokio::spawn(report_changes(
            path.to_owned(),
            dir,
            known,
            incoming,
            sink,
            client,
        ));
        Ok(Watch {
            watcher: Some(watcher),
            task,
        })
    }
}

/// Turn what the watcher saw into `FsChanged`: each entry that changed once
/// per burst, a create or remove judged by whether it is there now and was
/// before.
async fn report_changes(
    watch: String,
    dir: PathBuf,
    mut known: HashSet<PathBuf>,
    mut incoming: mpsc::UnboundedReceiver<Vec<PathBuf>>,
    sink: Sink,
    client: u64,
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
                FsChange::Remove
            } else if !exists {
                if !known.remove(&path) {
                    continue;
                }
                FsChange::Remove
            } else if known.insert(path.clone()) {
                FsChange::Create
            } else {
                FsChange::Modify
            };
            sink(Output::Send(
                client,
                server_message::Msg::FsChanged(FsChanged {
                    request_id: 0,
                    watch: watch.clone(),
                    path: path.to_string_lossy().into_owned(),
                    kind: kind.into(),
                }),
            ));
        }
    }
}

/// `~` and `~/...` made the home directory; anything else must be absolute.
fn expand(path: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let home = || home.ok_or_else(|| format!("{path}: no home directory"));
    if path == "~" {
        return Ok(home()?.to_path_buf());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return Ok(home()?.join(rest));
    }
    if Path::new(path).is_absolute() {
        Ok(PathBuf::from(path))
    } else {
        Err(format!("{path:?}: not an absolute path"))
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

fn kind_of(file_type: fs::FileType) -> FileKind {
    if file_type.is_symlink() {
        FileKind::Link
    } else if file_type.is_dir() {
        FileKind::Dir
    } else if file_type.is_file() {
        FileKind::File
    } else {
        FileKind::Other
    }
}

/// The entry at `path`, from its own metadata (a link's, not its target's).
fn entry_of(name: OsString, path: &Path, meta: &fs::Metadata) -> FsEntry {
    let kind = kind_of(meta.file_type());
    let target = match kind {
        FileKind::Link => match path.metadata() {
            Ok(meta) if meta.is_dir() => FileKind::Dir,
            Ok(meta) if meta.is_file() => FileKind::File,
            _ => FileKind::Other,
        },
        kind => kind,
    };
    FsEntry {
        name: name.to_string_lossy().into_owned(),
        kind: kind.into(),
        size: meta.len(),
        mtime_ms: meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_millis() as u64),
        mode: meta.mode() & 0o7777,
        target: target.into(),
    }
}

/// What went wrong with `path`, for the user.
fn failed(path: &str, err: &io::Error) -> String {
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
        assert_eq!(expand("~", Some(home)).unwrap(), home);
        assert_eq!(expand("~/a/b", Some(home)).unwrap(), home.join("a/b"));
        assert_eq!(expand("/etc", Some(home)).unwrap(), Path::new("/etc"));
        assert!(expand("a/b", Some(home)).is_err());
        assert!(expand("~other", Some(home)).is_err());
        assert!(expand("", Some(home)).is_err());
        assert!(expand("~", None).is_err());
    }

    #[test]
    fn dot_dot_takes_the_component_before_it() {
        assert_eq!(normalize(Path::new("/a/b/../c/./d")), Path::new("/a/c/d"));
        assert_eq!(normalize(Path::new("/a/../..")), Path::new("/"));
    }
}
