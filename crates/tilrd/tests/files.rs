//! Use cases: the files of the server's machine, for a file browser and an
//! editor; and `tilr edit` in a pane, which asks the client to open a file.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::*;
use tilr_proto::{
    FileKind, FsChange, FsChanged, FsDelete, FsEntry, FsList, FsMkdir, FsRead, FsRename, FsStat,
    FsUnwatch, FsWatch, FsWrite, HostEvent,
};

fn start_with_config(config: &str, args: &[&str]) -> Server {
    start_in_with_config(Server::scratch_dir(), config, args)
}

/// Like `start_with_config`, in `dir`: the server looks its roots up as it
/// starts, so they must exist by then.
fn start_in_with_config(dir: PathBuf, config: &str, args: &[&str]) -> Server {
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/server.toml"), config).unwrap();
    Server::start_in(dir, Path::new(env!("CARGO_BIN_EXE_tilrd")), args)
}

/// The server's home directory as it names it: without links (on macOS the
/// temporary directory is behind one).
fn home(server: &Server) -> PathBuf {
    std::fs::canonicalize(&server.dir).unwrap()
}

/// The answer to the request with `request_id`.
async fn answer(client: &mut Client, request_id: u64) -> server_message::Msg {
    client
        .wait_for(|msg| {
            let id = match msg {
                server_message::Msg::FsEntries(m) => m.request_id,
                server_message::Msg::FsInfo(m) => m.request_id,
                server_message::Msg::FsData(m) => m.request_id,
                server_message::Msg::FsAck(m) => m.request_id,
                _ => return None,
            };
            (id == request_id).then(|| msg.clone())
        })
        .await
}

async fn list(
    client: &mut Client,
    request_id: u64,
    path: &str,
) -> Result<(String, Vec<FsEntry>), String> {
    client
        .send(Msg::FsList(FsList {
            request_id,
            path: path.into(),
            ..Default::default()
        }))
        .await;
    match answer(client, request_id).await {
        server_message::Msg::FsEntries(e) if e.error.is_empty() => Ok((e.path, e.entries)),
        server_message::Msg::FsEntries(e) => Err(e.error),
        other => panic!("not an FsEntries: {other:?}"),
    }
}

async fn stat(
    client: &mut Client,
    request_id: u64,
    path: &str,
) -> Result<(String, FsEntry), String> {
    client
        .send(Msg::FsStat(FsStat {
            request_id,
            path: path.into(),
            ..Default::default()
        }))
        .await;
    match answer(client, request_id).await {
        server_message::Msg::FsInfo(i) if i.error.is_empty() => Ok((i.path, i.entry.unwrap())),
        server_message::Msg::FsInfo(i) => Err(i.error),
        other => panic!("not an FsInfo: {other:?}"),
    }
}

/// Read a file; also returns the number of chunks it came in.
async fn read(
    client: &mut Client,
    request_id: u64,
    path: &str,
    offset: u64,
    length: u64,
) -> Result<(Vec<u8>, usize), String> {
    client
        .send(Msg::FsRead(FsRead {
            request_id,
            path: path.into(),
            offset,
            length,
            ..Default::default()
        }))
        .await;
    let mut data = Vec::new();
    let mut chunks = 0;
    loop {
        let server_message::Msg::FsData(chunk) = answer(client, request_id).await else {
            panic!("not an FsData");
        };
        if !chunk.error.is_empty() {
            return Err(chunk.error);
        }
        chunks += 1;
        data.extend_from_slice(&chunk.data);
        assert!(data.len() as u64 <= chunk.total_size);
        if chunk.last {
            assert_eq!(data.len() as u64, chunk.total_size);
            return Ok((data, chunks));
        }
    }
}

/// Send a write in chunks, without waiting for the outcome.
async fn send_write(
    client: &mut Client,
    request_id: u64,
    path: &str,
    data: &[u8],
    create: bool,
    truncate: bool,
) {
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(CHUNK).collect()
    };
    for (index, chunk) in chunks.iter().enumerate() {
        let first = index == 0;
        client
            .send(Msg::FsWrite(FsWrite {
                request_id,
                transfer_id: request_id,
                path: if first { path.into() } else { String::new() },
                total_size: if first { data.len() as u64 } else { 0 },
                create: first && create,
                truncate: first && truncate,
                data: Bytes::copy_from_slice(chunk),
                last: index + 1 == chunks.len(),
                ..Default::default()
            }))
            .await;
    }
}

/// The outcome of a request answered with FsAck: empty when done.
async fn ack(client: &mut Client, request_id: u64) -> String {
    match answer(client, request_id).await {
        server_message::Msg::FsAck(ack) => ack.error,
        other => panic!("not an FsAck: {other:?}"),
    }
}

async fn write(
    client: &mut Client,
    request_id: u64,
    path: &str,
    data: &[u8],
    create: bool,
    truncate: bool,
) -> String {
    send_write(client, request_id, path, data, create, truncate).await;
    ack(client, request_id).await
}

async fn mkdir(client: &mut Client, request_id: u64, path: &str) -> String {
    let path = path.into();
    client
        .send(Msg::FsMkdir(FsMkdir {
            request_id,
            path,
            ..Default::default()
        }))
        .await;
    ack(client, request_id).await
}

async fn rename(client: &mut Client, request_id: u64, from: &str, to: &str) -> String {
    client
        .send(Msg::FsRename(FsRename {
            request_id,
            from: from.into(),
            to: to.into(),
            ..Default::default()
        }))
        .await;
    ack(client, request_id).await
}

async fn delete(client: &mut Client, request_id: u64, path: &str, recursive: bool) -> String {
    client
        .send(Msg::FsDelete(FsDelete {
            request_id,
            path: path.into(),
            recursive,
            ..Default::default()
        }))
        .await;
    ack(client, request_id).await
}

/// The next change pushed for `path`. File systems report some changes
/// more than once (the directory's own times, a write in two steps); what
/// is about other paths is passed over.
async fn changed(client: &mut Client, path: &str) -> FsChanged {
    client
        .wait_for(|msg| match msg {
            server_message::Msg::FsChanged(changed) if changed.path == path => {
                Some(changed.clone())
            }
            _ => None,
        })
        .await
}

/// Files whose name says they are a write in progress.
fn temp_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tilr-write-"))
        .collect()
}

#[tokio::test]
async fn a_client_lists_a_directory_and_looks_at_its_entries() {
    let server = Server::start(&[]);
    let docs = server.dir.join("docs");
    std::fs::create_dir_all(docs.join("sub")).unwrap();
    std::fs::write(docs.join("b.txt"), "twelve bytes").unwrap();
    std::fs::set_permissions(docs.join("b.txt"), std::fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink("b.txt", docs.join("a-link")).unwrap();
    std::os::unix::fs::symlink("nowhere", docs.join("broken")).unwrap();

    let mut client = Client::connect_silent(&server).await;
    let hello = client.hello().await;
    assert!(
        hello.features.contains(&"files".to_owned()),
        "{:?}",
        hello.features
    );
    assert_eq!(hello.fs_max_bytes, 64 << 20);

    let (path, entries) = list(&mut client, 1, "~/docs").await.unwrap();
    assert_eq!(Path::new(&path), home(&server).join("docs"));
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a-link", "b.txt", "broken", "sub"]);
    let kinds: Vec<_> = entries.iter().map(|e| (e.kind(), e.target())).collect();
    assert_eq!(
        kinds,
        [
            (FileKind::Link, FileKind::File),
            (FileKind::File, FileKind::File),
            (FileKind::Link, FileKind::Other),
            (FileKind::Dir, FileKind::Dir),
        ]
    );
    let file = &entries[1];
    assert_eq!(file.size, 12);
    assert_eq!(file.mode, 0o640);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert!(file.mtime_ms > now_ms - 60_000 && file.mtime_ms <= now_ms + 1000);

    // The same by absolute path, and through `..`.
    let absolute = format!("{}/docs/sub/..", server.dir.display());
    assert_eq!(list(&mut client, 2, &absolute).await.unwrap().1, entries);

    // A link is looked at itself, not followed; a root can be looked at too.
    let (_, link) = stat(&mut client, 3, "~/docs/a-link").await.unwrap();
    assert_eq!(
        (link.name.as_str(), link.kind(), link.target()),
        ("a-link", FileKind::Link, FileKind::File)
    );
    let (path, root) = stat(&mut client, 4, "~").await.unwrap();
    assert_eq!(Path::new(&path), home(&server));
    assert_eq!(root.kind(), FileKind::Dir);
    // Listing through a link goes to what it points to.
    std::os::unix::fs::symlink("docs/sub", server.dir.join("to-sub")).unwrap();
    std::fs::write(docs.join("sub/inner"), "").unwrap();
    let (path, entries) = list(&mut client, 5, "~/to-sub").await.unwrap();
    assert_eq!(Path::new(&path), home(&server).join("docs/sub"));
    assert_eq!(entries[0].name, "inner");

    let error = list(&mut client, 6, "~/missing").await.unwrap_err();
    assert_eq!(error, "~/missing: no such file or directory");
    let error = list(&mut client, 7, "~/docs/b.txt").await.unwrap_err();
    assert!(error.contains("not a directory"), "{error}");
    let error = stat(&mut client, 8, "docs").await.unwrap_err();
    assert!(error.contains("not an absolute path"), "{error}");
}

/// Linux allows any bytes but '/' and NUL in a name (macOS file systems
/// take UTF-8 only).
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_name_that_is_not_utf8_is_listed_with_its_bytes_and_named_by_them() {
    use std::os::unix::ffi::OsStrExt;

    let server = Server::start(&[]);
    let dir = server.dir.join("odd");
    std::fs::create_dir_all(&dir).unwrap();
    let raw = b"bin\xff\xfe-\xc3(name.dat";
    let name = std::ffi::OsStr::from_bytes(raw);
    std::fs::write(dir.join(name), "odd content").unwrap();
    std::fs::write(dir.join("plain.txt"), "plain").unwrap();
    let mut client = Client::connect_silent(&server).await;
    client.hello().await;

    let (_, entries) = list(&mut client, 1, "~/odd").await.unwrap();
    let odd = entries.iter().find(|e| e.name != "plain.txt").unwrap();
    assert_eq!(
        odd.name, "bin\u{fffd}\u{fffd}-\u{fffd}(name.dat",
        "shown lossily"
    );
    assert_eq!(odd.name_bytes, raw, "and named exactly by its bytes");
    let plain = entries.iter().find(|e| e.name == "plain.txt").unwrap();
    assert!(
        plain.name_bytes.is_empty(),
        "a UTF-8 name has no bytes twin"
    );

    let mut path = home(&server).join("odd").as_os_str().as_bytes().to_vec();
    path.extend_from_slice(b"/");
    path.extend_from_slice(raw);
    client
        .send(Msg::FsRead(FsRead {
            request_id: 2,
            path: odd.name.clone(),
            path_bytes: path.clone(),
            ..Default::default()
        }))
        .await;
    let server_message::Msg::FsData(data) = answer(&mut client, 2).await else {
        panic!("not an FsData");
    };
    assert_eq!(
        (data.error.as_str(), &data.data[..]),
        ("", &b"odd content"[..])
    );

    client
        .send(Msg::FsStat(FsStat {
            request_id: 3,
            path: String::new(),
            path_bytes: path.clone(),
        }))
        .await;
    let server_message::Msg::FsInfo(info) = answer(&mut client, 3).await else {
        panic!("not an FsInfo");
    };
    assert_eq!(info.error, "");
    assert_eq!(info.path_bytes, path, "the path answered has its bytes too");
    assert_eq!(info.entry.unwrap().name_bytes, raw);

    let mut renamed = path.clone();
    renamed.extend_from_slice(b"\xff");
    client
        .send(Msg::FsRename(FsRename {
            request_id: 4,
            from_bytes: path.clone(),
            to_bytes: renamed.clone(),
            ..Default::default()
        }))
        .await;
    assert_eq!(ack(&mut client, 4).await, "");
    assert!(Path::new(std::ffi::OsStr::from_bytes(&renamed)).exists());
    client
        .send(Msg::FsDelete(FsDelete {
            request_id: 5,
            path_bytes: renamed.clone(),
            ..Default::default()
        }))
        .await;
    assert_eq!(ack(&mut client, 5).await, "");
    let (_, entries) = list(&mut client, 6, "~/odd").await.unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["plain.txt"]);
}

#[tokio::test]
async fn a_file_is_read_in_chunks_whole_or_in_part() {
    let server = start_with_config("[files]\nmax_bytes = \"600K\"\n", &[]);
    let mut client = Client::connect(&server).await;
    let data = image(2 * CHUNK + 12_345);
    std::fs::write(server.dir.join("big.bin"), &data).unwrap();

    let (got, chunks) = read(&mut client, 1, "~/big.bin", 0, 0).await.unwrap();
    assert_eq!(got, data);
    assert_eq!(chunks, 3);

    let (got, _) = read(&mut client, 2, "~/big.bin", 1000, 5000).await.unwrap();
    assert_eq!(got, &data[1000..6000]);
    // A part that runs past the end ends there; past the end is nothing.
    let (got, _) = read(&mut client, 3, "~/big.bin", data.len() as u64 - 10, 100)
        .await
        .unwrap();
    assert_eq!(got, &data[data.len() - 10..]);
    let (got, chunks) = read(&mut client, 4, "~/big.bin", data.len() as u64 + 5, 0)
        .await
        .unwrap();
    assert!(got.is_empty());
    assert_eq!(chunks, 1);
    std::fs::write(server.dir.join("empty"), "").unwrap();
    assert!(
        read(&mut client, 5, "~/empty", 0, 0)
            .await
            .unwrap()
            .0
            .is_empty()
    );

    // More than the limit at once is refused; in parts it can be had.
    let huge = image(700 << 10);
    std::fs::write(server.dir.join("huge.bin"), &huge).unwrap();
    let error = read(&mut client, 6, "~/huge.bin", 0, 0).await.unwrap_err();
    assert!(
        error.contains("too large to read at once: 700 KB"),
        "{error}"
    );
    assert!(error.contains("at most 600 KB"), "{error}");
    let (got, _) = read(&mut client, 7, "~/huge.bin", 400 << 10, 0)
        .await
        .unwrap();
    assert_eq!(got, &huge[400 << 10..]);

    let error = read(&mut client, 8, "~", 0, 0).await.unwrap_err();
    assert_eq!(error, "~: is a directory");
    let error = read(&mut client, 9, "~/nope", 0, 0).await.unwrap_err();
    assert_eq!(error, "~/nope: no such file or directory");
}

#[tokio::test]
async fn a_file_is_written_whole_or_not_at_all() {
    let server = start_with_config("[files]\nmax_bytes = \"1MB\"\n", &[]);
    let mut client = Client::connect(&server).await;
    let file = server.dir.join("notes.md");

    // A new file, in several chunks.
    let data = image(2 * CHUNK + 777);
    assert_eq!(
        write(&mut client, 1, "~/notes.md", &data, true, false).await,
        ""
    );
    assert_eq!(std::fs::read(&file).unwrap(), data);
    // `create` alone makes new files only.
    let error = write(&mut client, 2, "~/notes.md", b"x", true, false).await;
    assert_eq!(error, "~/notes.md: already exists");
    // `truncate` replaces; the file keeps its permissions.
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        write(&mut client, 3, "~/notes.md", b"saved", false, true).await,
        ""
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"saved");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        write(&mut client, 4, "~/notes.md", b"", false, true).await,
        ""
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"");
    let error = write(&mut client, 5, "~/new.md", b"x", false, true).await;
    assert_eq!(error, "~/new.md: no such file");
    let error = write(&mut client, 6, "~/no-dir/new.md", b"x", true, true).await;
    assert_eq!(error, "~/no-dir/new.md: no such file or directory");
    let error = write(&mut client, 7, "~", b"x", true, true).await;
    assert!(error.contains("outside"), "{error}");
    std::fs::create_dir(server.dir.join("dir")).unwrap();
    let error = write(&mut client, 8, "~/dir", b"x", true, true).await;
    assert_eq!(error, "~/dir: is a directory");

    // A link is written through: the file it points to changes.
    std::os::unix::fs::symlink("notes.md", server.dir.join("link.md")).unwrap();
    assert_eq!(
        write(&mut client, 9, "~/link.md", b"through", false, true).await,
        ""
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"through");
    assert!(
        std::fs::symlink_metadata(server.dir.join("link.md"))
            .unwrap()
            .is_symlink()
    );

    // Over the limit: refused on the first chunk, the rest ignored.
    send_write(
        &mut client,
        10,
        "~/notes.md",
        &image(1100 << 10),
        false,
        true,
    )
    .await;
    let error = ack(&mut client, 10).await;
    assert!(error.contains("too large: 1.1 MB"), "{error}");
    // A write that sends less than it announced, or more, changes nothing.
    for (request_id, sent) in [(11u64, 50usize), (12, 150)] {
        client
            .send(Msg::FsWrite(FsWrite {
                request_id,
                transfer_id: request_id,
                path: "~/notes.md".into(),
                total_size: 100,
                truncate: true,
                data: Bytes::from(image(sent)),
                last: true,
                ..Default::default()
            }))
            .await;
        let error = ack(&mut client, request_id).await;
        assert!(
            error.contains("of 100 bytes") || error.contains("more data"),
            "{error}"
        );
    }
    assert_eq!(std::fs::read(&file).unwrap(), b"through");
    // Nothing else was sent for the refused ones: the next answer is this.
    assert_eq!(
        write(&mut client, 13, "~/other", b"1", true, false).await,
        ""
    );

    // A client that leaves in the middle of a write leaves the file as it
    // was, and no temporary file behind.
    send_write(
        &mut client,
        14,
        "~/notes.md",
        &image(3 * CHUNK),
        false,
        true,
    )
    .await;
    client
        .send(Msg::FsWrite(FsWrite {
            request_id: 15,
            transfer_id: 15,
            path: "~/notes.md".into(),
            total_size: 2 * CHUNK as u64,
            truncate: true,
            data: Bytes::from(image(CHUNK)),
            last: false,
            ..Default::default()
        }))
        .await;
    assert_eq!(ack(&mut client, 14).await, "");
    assert_eq!(std::fs::read(&file).unwrap(), image(3 * CHUNK));
    eventually("the half-written file appears", || {
        !temp_files(&server.dir).is_empty()
    })
    .await;
    drop(client);
    eventually("the half-written file is removed", || {
        temp_files(&server.dir).is_empty()
    })
    .await;
    assert_eq!(std::fs::read(&file).unwrap(), image(3 * CHUNK));
}

#[tokio::test]
async fn directories_are_made_entries_renamed_and_deleted() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let home = server.dir.clone();

    assert_eq!(mkdir(&mut client, 1, "~/project").await, "");
    assert!(home.join("project").is_dir());
    assert_eq!(
        mkdir(&mut client, 2, "~/project").await,
        "~/project: already exists"
    );
    assert_eq!(
        mkdir(&mut client, 3, "~/a/b").await,
        "~/a/b: no such file or directory"
    );

    std::fs::write(home.join("project/draft.md"), "draft").unwrap();
    assert_eq!(
        rename(&mut client, 4, "~/project/draft.md", "~/project/final.md").await,
        ""
    );
    assert_eq!(
        std::fs::read_to_string(home.join("project/final.md")).unwrap(),
        "draft"
    );
    assert!(!home.join("project/draft.md").exists());
    // A rename does not overwrite.
    std::fs::write(home.join("project/other.md"), "other").unwrap();
    let error = rename(&mut client, 5, "~/project/other.md", "~/project/final.md").await;
    assert_eq!(error, "~/project/final.md: already exists");
    assert_eq!(
        std::fs::read_to_string(home.join("project/final.md")).unwrap(),
        "draft"
    );
    // Directories move too.
    assert_eq!(rename(&mut client, 6, "~/project", "~/moved").await, "");
    assert!(home.join("moved/final.md").is_file());
    let error = rename(&mut client, 7, "~/nothing", "~/something").await;
    assert_eq!(error, "~/nothing: no such file or directory");

    // A link goes, what it points to stays.
    std::os::unix::fs::symlink("moved/final.md", home.join("link")).unwrap();
    assert_eq!(delete(&mut client, 8, "~/link", false).await, "");
    assert!(std::fs::symlink_metadata(home.join("link")).is_err());
    assert!(home.join("moved/final.md").is_file());
    assert_eq!(delete(&mut client, 9, "~/moved/other.md", false).await, "");
    assert!(!home.join("moved/other.md").exists());
    // A directory with something in it goes only when asked to.
    let error = delete(&mut client, 10, "~/moved", false).await;
    assert_eq!(error, "~/moved: directory not empty");
    assert_eq!(delete(&mut client, 11, "~/moved", true).await, "");
    assert!(!home.join("moved").exists());
    std::fs::create_dir(home.join("empty")).unwrap();
    assert_eq!(delete(&mut client, 12, "~/empty", false).await, "");
    assert_eq!(
        delete(&mut client, 13, "~/empty", false).await,
        "~/empty: no such file or directory"
    );
}

#[tokio::test]
async fn a_watched_directory_reports_what_changes_in_it() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let dir = server.dir.join("watched");
    std::fs::create_dir_all(dir.join("deeper")).unwrap();
    std::fs::write(dir.join("old.txt"), "old").unwrap();
    let real = home(&server).join("watched");
    let path_of = |name: &str| real.join(name).to_string_lossy().into_owned();

    client
        .send(Msg::FsWatch(FsWatch {
            request_id: 1,
            path: "~/watched".into(),
            ..Default::default()
        }))
        .await;
    assert_eq!(ack(&mut client, 1).await, "");

    std::fs::write(dir.join("new.txt"), "new").unwrap();
    let change = changed(&mut client, &path_of("new.txt")).await;
    assert_eq!(change.request_id, 0);
    assert_eq!(change.watch, "~/watched");
    assert_eq!(change.kind(), FsChange::Create);

    std::fs::write(dir.join("old.txt"), "changed").unwrap();
    let change = changed(&mut client, &path_of("old.txt")).await;
    assert_eq!(change.kind(), FsChange::Modify);

    std::fs::remove_file(dir.join("new.txt")).unwrap();
    let change = changed(&mut client, &path_of("new.txt")).await;
    assert_eq!(change.kind(), FsChange::Remove);

    // A save through the files protocol is a change of the file; the
    // temporary file it went through is not reported.
    assert_eq!(
        write(&mut client, 2, "~/watched/old.txt", b"saved", false, true).await,
        ""
    );
    let change = changed(&mut client, &path_of("old.txt")).await;
    assert_eq!(change.kind(), FsChange::Modify);

    // Subdirectories are not watched; a new one in the directory is seen.
    std::fs::write(dir.join("deeper/inside"), "x").unwrap();
    std::fs::create_dir(dir.join("made")).unwrap();
    let change = changed(&mut client, &path_of("made")).await;
    assert_eq!(change.kind(), FsChange::Create);

    client
        .send(Msg::FsUnwatch(FsUnwatch {
            request_id: 3,
            path: "~/watched".into(),
            ..Default::default()
        }))
        .await;
    assert_eq!(ack(&mut client, 3).await, "");
    client
        .send(Msg::FsUnwatch(FsUnwatch {
            request_id: 4,
            path: "~/watched".into(),
            ..Default::default()
        }))
        .await;
    assert_eq!(ack(&mut client, 4).await, "~/watched: not watched");
    std::fs::write(dir.join("after.txt"), "x").unwrap();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    client.ping(9).await;
    let after = path_of("after.txt");
    let next = client
        .wait_for(|msg| match msg {
            server_message::Msg::FsChanged(changed) if changed.path == after => {
                Some(Err(changed.clone()))
            }
            server_message::Msg::Pong(_) => Some(Ok(())),
            _ => None,
        })
        .await;
    assert_eq!(next, Ok(()), "a change reported after unwatching");
    // The save left no temporary file behind.
    assert!(temp_files(&dir).is_empty());

    client
        .send(Msg::FsWatch(FsWatch {
            request_id: 5,
            path: "~/watched/old.txt".into(),
            ..Default::default()
        }))
        .await;
    assert!(ack(&mut client, 5).await.contains("not a directory"));
}

#[tokio::test]
async fn nothing_outside_the_shared_directories_is_served() {
    let home = Server::scratch_dir();
    std::fs::create_dir_all(home.join("shared/inner")).unwrap();
    std::fs::create_dir_all(home.join("secret")).unwrap();
    std::fs::write(home.join("secret/key"), "key").unwrap();
    std::os::unix::fs::symlink("../secret", home.join("shared/door")).unwrap();
    std::os::unix::fs::symlink("../secret/key", home.join("shared/key-link")).unwrap();
    let server = start_in_with_config(home.clone(), "[files]\nroots = [\"~/shared\"]\n", &[]);
    let mut client = Client::connect(&server).await;

    let (_, entries) = list(&mut client, 1, "~/shared").await.unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["door", "inner", "key-link"]);

    fn outside<T>(error: Result<T, String>, path: &str) {
        let error = error.err().unwrap_or_default();
        assert_eq!(
            error,
            format!("{path}: outside the directories this server shares")
        );
    }
    outside(list(&mut client, 2, "~").await, "~");
    outside(
        list(&mut client, 3, "~/shared/../secret").await,
        "~/shared/../secret",
    );
    outside(list(&mut client, 4, "~/shared/door").await, "~/shared/door");
    outside(list(&mut client, 5, "/").await, "/");
    // What is or is not there outside is not told either.
    outside(list(&mut client, 6, "~/missing").await, "~/missing");
    outside(stat(&mut client, 7, "~/secret/key").await, "~/secret/key");
    outside(
        read(&mut client, 8, "~/shared/key-link", 0, 0).await,
        "~/shared/key-link",
    );
    outside(
        read(&mut client, 9, "~/shared/inner/../../secret/key", 0, 0).await,
        "~/shared/inner/../../secret/key",
    );
    let refused = |error: String, path: &str| {
        assert_eq!(
            error,
            format!("{path}: outside the directories this server shares")
        );
    };
    refused(
        write(&mut client, 10, "~/shared/key-link", b"x", true, true).await,
        "~/shared/key-link",
    );
    refused(
        write(&mut client, 11, "~/shared/../new", b"x", true, true).await,
        "~/shared/../new",
    );
    refused(mkdir(&mut client, 12, "~/secret/sub").await, "~/secret/sub");
    refused(
        rename(&mut client, 13, "~/shared/inner", "~/inner").await,
        "~/inner",
    );
    refused(
        rename(&mut client, 14, "~/secret/key", "~/shared/key").await,
        "~/secret/key",
    );
    // A root itself cannot be removed or moved.
    refused(delete(&mut client, 15, "~/shared", true).await, "~/shared");
    refused(
        rename(&mut client, 16, "~/shared", "~/shared2").await,
        "~/shared",
    );
    client
        .send(Msg::FsWatch(FsWatch {
            request_id: 17,
            path: "~/shared/door".into(),
            ..Default::default()
        }))
        .await;
    refused(ack(&mut client, 17).await, "~/shared/door");

    assert_eq!(
        std::fs::read_to_string(home.join("secret/key")).unwrap(),
        "key"
    );
    assert!(home.join("shared").is_dir() && home.join("secret").is_dir());
    // The link itself is in the shared directory and can go.
    assert_eq!(delete(&mut client, 18, "~/shared/door", false).await, "");
    assert!(home.join("secret/key").exists());
}

#[tokio::test]
async fn a_read_only_server_shows_files_but_changes_nothing() {
    let server = start_with_config("[files]\nreadonly = true\n", &[]);
    std::fs::write(server.dir.join("a.txt"), "content").unwrap();
    std::fs::create_dir(server.dir.join("dir")).unwrap();
    let mut client = Client::connect(&server).await;

    assert!(list(&mut client, 1, "~").await.is_ok());
    assert_eq!(
        read(&mut client, 2, "~/a.txt", 0, 0).await.unwrap().0,
        b"content"
    );
    let readonly = "files are read-only on this server";
    assert_eq!(
        write(&mut client, 3, "~/a.txt", &image(CHUNK + 1), false, true).await,
        readonly
    );
    assert_eq!(
        write(&mut client, 4, "~/b.txt", b"x", true, false).await,
        readonly
    );
    assert_eq!(mkdir(&mut client, 5, "~/new").await, readonly);
    assert_eq!(rename(&mut client, 6, "~/a.txt", "~/c.txt").await, readonly);
    assert_eq!(delete(&mut client, 7, "~/dir", true).await, readonly);
    assert_eq!(
        std::fs::read_to_string(server.dir.join("a.txt")).unwrap(),
        "content"
    );
    assert!(!server.dir.join("b.txt").exists() && !server.dir.join("new").exists());
    assert!(server.dir.join("dir").is_dir());
}

#[tokio::test]
async fn files_can_be_turned_off() {
    let flag = Server::start(&["--no-files"]);
    let config = start_with_config("[files]\nenabled = false\n", &[]);
    for server in [&flag, &config] {
        std::fs::write(server.dir.join("a.txt"), "content").unwrap();
        let mut client = Client::connect_silent(server).await;
        let hello = client.hello().await;
        assert!(
            !hello.features.contains(&"files".to_owned()),
            "{:?}",
            hello.features
        );
        assert_eq!(hello.fs_max_bytes, 0);

        // Requests go unanswered and change nothing; the connection stays.
        client
            .send(Msg::FsList(FsList {
                request_id: 1,
                path: "~".into(),
                ..Default::default()
            }))
            .await;
        send_write(&mut client, 2, "~/a.txt", b"x", true, true).await;
        client.ping(3).await;
        let next = client
            .wait_for(|msg| match msg {
                server_message::Msg::Pong(_) => Some(None),
                server_message::Msg::FsEntries(_)
                | server_message::Msg::FsAck(_)
                | server_message::Msg::Error(_) => Some(Some(format!("{msg:?}"))),
                _ => None,
            })
            .await;
        assert_eq!(next, None);
        assert_eq!(
            std::fs::read_to_string(server.dir.join("a.txt")).unwrap(),
            "content"
        );
    }
}

/// The oldest host event the client got and was not yet asked about.
async fn host_event(client: &mut Client) -> HostEvent {
    loop {
        if let Some(event) = client.host_events.pop_front() {
            return event;
        }
        client.next().await.expect("connection closed");
    }
}

#[tokio::test]
async fn tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file() {
    let server = Server::start(&[]);
    let mut client = Client::connect_silent(&server).await;
    let hello = client.hello().await;
    assert!(
        hello.features.contains(&"host-events".to_owned()),
        "{:?}",
        hello.features
    );
    let pane_id = client.open_pane().await;
    let mut other = Client::connect(&server).await;
    other.subscribe(pane_id).await;
    // A host event goes to the clients watching the pane when it comes. The
    // subscribe travels on a connection of its own, not ordered with
    // `client`'s input: `other` watches once its first frame is there.
    other
        .wait_for(|msg| matches!(msg, server_message::Msg::Frame(_)).then_some(()))
        .await;

    // The pane knows its server and how to reach it.
    let check = format!(
        "test \"$TILR_PANE\" = {pane_id} && test \"$TILR_SERVER\" = {} && test -S \"$TILR_SOCK\"",
        hello.server_id
    );
    assert_eq!(client.run(pane_id, &check).await, 0);

    std::fs::create_dir(server.dir.join("project")).unwrap();
    assert_eq!(
        client
            .run(pane_id, "cd ~/project && tilr edit notes.md")
            .await,
        0
    );
    for client in [&mut client, &mut other] {
        let event = host_event(client).await;
        assert_eq!(event.pane_id, pane_id.to_string());
        assert_eq!((event.name.as_str(), event.kind.as_str()), ("open", "file"));
        let path = Path::new(&event.path);
        assert_eq!(path.file_name().unwrap(), "notes.md");
        assert_eq!(
            std::fs::canonicalize(path.parent().unwrap()).unwrap(),
            home(&server).join("project")
        );
        assert!(event.args.is_empty());
    }

    let open = "tilr open https://example.com/a -- --float";
    assert_eq!(client.run(pane_id, open).await, 0);
    let event = host_event(&mut client).await;
    assert_eq!(
        (event.kind.as_str(), event.path.as_str()),
        ("url", "https://example.com/a")
    );
    assert_eq!(event.args, ["--float"]);
    let open = "tilr open --kind image ../x.png";
    assert_eq!(client.run(pane_id, open).await, 0);
    let event = host_event(&mut client).await;
    assert_eq!(
        (event.kind.as_str(), event.path.as_str()),
        ("image", "../x.png")
    );

    // Other commands are the business of another `tilr`, if there is one.
    let out = server.dir.join("other.txt");
    let other_command = format!("tilr ctl apps 2> '{}'", out.display());
    assert_ne!(client.run(pane_id, &other_command).await, 0);
    let said = std::fs::read_to_string(&out).unwrap();
    assert!(said.contains("no other tilr is installed"), "{said}");
}

#[tokio::test]
async fn tilr_edit_fails_with_no_client_watching_or_outside_a_pane() {
    let mut server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let sock_file = server.dir.join("sock.txt");
    let save = format!("printf %s \"$TILR_SOCK\" > '{}'", sock_file.display());
    assert_eq!(client.run(pane_id, &save).await, 0);
    let sock = std::fs::read_to_string(&sock_file).unwrap();
    client
        .send(Msg::Unsubscribe(Unsubscribe {
            pane_id: pane_id.to_string(),
        }))
        .await;
    // Unsubscribing is done once the pane list says nobody watches.
    while client.pushed_pane_list().await[0].viewers != 0 {}
    // A client that watches the pane but did not announce "host-events"
    // (one built before them) does not count.
    let mut old = Client::connect_silent(&server).await;
    old.hello_with(PROTOCOL_VERSION, &["clipboard"]).await;
    old.subscribe(pane_id).await;
    while client.pushed_pane_list().await[0].viewers != 1 {}

    let tilr = |env: &[(&str, &str)]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tilrd"));
        command
            .args(["edit", "notes.md"])
            .env_remove("TILR_PANE")
            .env_remove("TILR_SOCK")
            .envs(env.iter().copied());
        let output = command.output().unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let pane = pane_id.to_string();
    assert_eq!(
        tilr(&[("TILR_PANE", &pane), ("TILR_SOCK", &sock)]),
        (Some(1), "tilr: no tilr client attached\n".into())
    );
    assert_eq!(
        tilr(&[]),
        (Some(1), "tilr: not inside a tilr pane\n".into())
    );
    let unknown = "00000000-0000-4000-8000-000000000000";
    let (code, said) = tilr(&[("TILR_PANE", unknown), ("TILR_SOCK", &sock)]);
    assert_eq!(
        (code, said.as_str()),
        (Some(1), format!("tilr: no such pane: {unknown}\n").as_str())
    );

    // The socket speaks JSON lines.
    {
        use std::io::Write;
        let mut stream = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        stream.write_all(b"not json\n").unwrap();
        let mut answer = String::new();
        BufReader::new(&stream).read_line(&mut answer).unwrap();
        assert!(
            answer.starts_with("{\"ok\":false,\"error\":\"bad request"),
            "{answer}"
        );
    }

    // Stopping the server takes the socket away.
    assert!(Path::new(&sock).exists());
    server.signal("-TERM");
    for _ in 0..100 {
        if !server.is_running() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!server.is_running());
    assert!(!Path::new(&sock).exists());
}

/// A way to `server` for a client that answers only the latest of the
/// pings it has read, as RFC 6455 allows: of the pongs the client sends, the
/// proxy holds the newest and lets it through every 300 ms, dropping the
/// others. Returns the address to connect to.
async fn latest_pong_only(server: &Server) -> std::net::SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = server.addr.trim_start_matches("ws://").to_owned();
    tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let server = TcpStream::connect(upstream).await.unwrap();
        let (mut from_client, mut to_client) = client.into_split();
        let (mut from_server, mut to_server) = server.into_split();
        tokio::spawn(async move {
            let _ = tokio::io::copy(&mut from_server, &mut to_client).await;
        });
        // The handshake request goes as it is.
        let mut buf = Vec::new();
        let mut read = [0; 16 << 10];
        let end = loop {
            if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
            match from_client.read(&mut read).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&read[..n]),
            }
        };
        if to_server.write_all(&buf[..end]).await.is_err() {
            return;
        }
        buf.drain(..end);
        // Then frames from the client, whole: a client masks every frame.
        let mut held: Option<Vec<u8>> = None;
        let mut flush = tokio::time::interval(Duration::from_millis(300));
        loop {
            tokio::select! {
                n = from_client.read(&mut read) => match n {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&read[..n]),
                },
                _ = flush.tick() => {
                    if let Some(pong) = held.take()
                        && to_server.write_all(&pong).await.is_err()
                    {
                        return;
                    }
                    continue;
                }
            }
            while buf.len() >= 2 {
                let (len, head) = match buf[1] & 0x7f {
                    126 if buf.len() >= 4 => (u16::from_be_bytes([buf[2], buf[3]]) as usize, 4),
                    127 if buf.len() >= 10 => (
                        u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize,
                        10,
                    ),
                    126 | 127 => break,
                    len => (len as usize, 2),
                };
                let size = head + 4 + len;
                if buf.len() < size {
                    break;
                }
                let frame: Vec<u8> = buf.drain(..size).collect();
                if frame[0] & 0x0f == 0xa {
                    held = Some(frame);
                } else if to_server.write_all(&frame).await.is_err() {
                    return;
                }
            }
        }
    });
    addr
}

#[tokio::test]
async fn a_client_answering_only_the_latest_ping_gets_the_whole_file() {
    // Liveness pings every third of a second, among the bulk window's marks.
    let server = Server::start(&["--client-timeout", "1"]);
    let data = image(16 << 20);
    std::fs::write(server.dir.join("big.bin"), &data).unwrap();
    let stream = TcpStream::connect(latest_pong_only(&server).await)
        .await
        .unwrap();
    let (ws, _) = tokio_tungstenite::client_async(&server.addr, MaybeTlsStream::Plain(stream))
        .await
        .unwrap();
    let mut client = Client::connect_silent(&server).await;
    client.ws = ws;
    client.hello().await;

    let (got, _) = read(&mut client, 1, "~/big.bin", 0, 0).await.unwrap();
    assert!(got == data, "the file arrived changed");
}

/// `tilr edit` in the pane reaches `client`, which watches it.
async fn tilr_edit_reaches(client: &mut Client, pane_id: PaneId, name: &str) {
    assert_eq!(client.run(pane_id, &format!("tilr edit {name}")).await, 0);
    let event = host_event(client).await;
    assert_eq!(event.pane_id, pane_id.to_string());
    assert_eq!(Path::new(&event.path).file_name().unwrap(), name);
}

#[tokio::test]
async fn two_servers_of_one_user_keep_their_host_sockets() {
    let first = Server::start(&[]);
    let mut second = start_second(&first.dir);
    let mut a = Client::connect_silent(&first).await;
    let server_id = a.hello().await.server_id;
    let mut b = Client::connect(&second).await;
    let pane_a = a.open_pane().await;
    let pane_b = b.open_pane().await;

    let sock_a = pane_sock(&first, &mut a, pane_a).await;
    let sock_b = pane_sock(&second, &mut b, pane_b).await;
    assert_eq!(
        sock_a.file_name().unwrap(),
        format!("{server_id}.sock").as_str()
    );
    assert_eq!(
        sock_b.file_name().unwrap(),
        format!("{server_id}-{}.sock", second.child.id()).as_str()
    );
    tilr_edit_reaches(&mut a, pane_a, "a.md").await;
    tilr_edit_reaches(&mut b, pane_b, "b.md").await;

    // The second stops: it takes its own socket, not the first one's.
    stop(&mut second).await;
    assert!(!sock_b.exists(), "the second server left its socket");
    assert!(
        sock_a.exists(),
        "the second server removed the first one's socket"
    );
    tilr_edit_reaches(&mut a, pane_a, "again.md").await;
}
