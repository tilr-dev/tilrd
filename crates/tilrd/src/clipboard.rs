//! The server's clipboard: content a client uploads with `ClipboardPut` so
//! that programs in the panes can paste it.
//!
//! A program on a remote machine has no way to reach the clipboard of the
//! user's computer; it asks the local one, usually by running `xclip` or
//! `wl-paste`. The server therefore keeps the uploaded content in a file and
//! puts its own `xclip` and `wl-paste` first on the PATH of every pane: this
//! executable under another name (see [`shim`]), answering from that file.
//!
//! The clipboard is one per user and machine, shared by all panes. Any shell
//! of the same user can join in by putting [`shim_dir`] on its PATH.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use tilr_proto::{ClipboardAck, ClipboardPut};

/// Names this executable answers to as a clipboard tool.
pub const SHIMS: [&str; 2] = ["xclip", "wl-paste"];
/// Tells the shims where the clipboard is. Set for every pane.
pub const DIR_ENV: &str = "TILR_CLIPBOARD_DIR";
/// The directory of the tools the server puts first on the PATH of every pane
/// (`xclip`, `wl-paste`, `tilr`). A pane runs a login shell, whose profile
/// may set PATH from scratch (Debian's /etc/profile, a `PATH=...` without
/// `$PATH` in ~/.profile); `PATH="$TILR_SHIMS:$PATH"` there brings them back.
pub const SHIMS_ENV: &str = "TILR_SHIMS";
const CONTENT_FILE: &str = "content";
const MAX_MIME_LEN: usize = 127;

/// Directory for the server's per-user files, private to the user; see
/// [`tilr_proto::local::runtime_dir`].
pub fn runtime_dir() -> PathBuf {
    tilr_proto::local::runtime_dir()
}

pub fn clipboard_dir(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("clipboard")
}

pub fn shim_dir(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("bin")
}

/// Create `dir` readable by its owner only, and make sure it is ours: under
/// a shared /tmp somebody else could have made it first.
pub fn private_dir(dir: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("create {}", dir.display()))?;
    let meta = fs::metadata(dir).with_context(|| format!("stat {}", dir.display()))?;
    // SAFETY: geteuid has no preconditions.
    if meta.uid() != unsafe { libc::geteuid() } {
        bail!("{} belongs to another user", dir.display());
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("restrict {}", dir.display()))
}

/// Put links to `exe` named `names` into the shim directory and return it.
/// Those of `all` not in `names`, left by an earlier run, are removed.
pub fn install_shims(
    runtime_dir: &Path,
    exe: &Path,
    names: &[&str],
    all: &[&str],
) -> Result<PathBuf> {
    private_dir(runtime_dir)?;
    let dir = shim_dir(runtime_dir);
    private_dir(&dir)?;
    for name in all {
        let link = dir.join(name);
        match fs::remove_file(&link) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => {
                return Err(err).with_context(|| format!("replace {}", link.display()));
            }
            _ => {}
        }
        if !names.contains(name) {
            continue;
        }
        std::os::unix::fs::symlink(exe, &link)
            .with_context(|| format!("link {}", link.display()))?;
    }
    Ok(dir)
}

/// What the clipboard holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Content {
    pub mime: String,
    pub data: Vec<u8>,
}

/// The clipboard on disk: one file holding the MIME type, the time the
/// content expires (Unix milliseconds, 0 = never) and the content itself,
/// replaced as a whole.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The store in `dir`, which is created if missing.
    pub fn create(dir: PathBuf) -> Result<Self> {
        private_dir(&dir)?;
        Ok(Self { dir })
    }

    /// The store in `dir`, for reading.
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn file(&self) -> PathBuf {
        self.dir.join(CONTENT_FILE)
    }

    pub fn clear(&self) {
        let _ = fs::remove_file(self.file());
    }

    /// Replace the content. Readers see the old content or the new, never a
    /// mix. `ttl` of `None` keeps it until replaced.
    pub fn publish(&self, mime: &str, data: &[u8], ttl: Option<Duration>) -> io::Result<()> {
        let expires = ttl.map_or(0, |ttl| unix_now_ms() + ttl.as_millis() as u64);
        let tmp = self
            .dir
            .join(format!("{CONTENT_FILE}.{}", std::process::id()));
        let written = (|| {
            let mut file = fs::File::create(&tmp)?;
            writeln!(file, "{mime}\n{expires}")?;
            file.write_all(data)?;
            fs::rename(&tmp, self.file())
        })();
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written
    }

    /// The content, unless there is none or it has expired.
    pub fn read(&self) -> Option<Content> {
        let raw = fs::read(self.file()).ok()?;
        let mut parts = raw.splitn(3, |&byte| byte == b'\n');
        let mime = std::str::from_utf8(parts.next()?).ok()?.to_owned();
        let expires: u64 = std::str::from_utf8(parts.next()?).ok()?.parse().ok()?;
        let data = parts.next()?.to_vec();
        if expires != 0 && unix_now_ms() >= expires {
            return None;
        }
        Some(Content { mime, data })
    }
}

/// Milliseconds since the Unix epoch.
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

// ---------------------------------------------------------------------------
// Receiving ClipboardPut
// ---------------------------------------------------------------------------

pub struct Limits {
    pub max_bytes: u64,
    /// How long content stays readable. `None` = until replaced.
    pub ttl: Option<Duration>,
}

enum Transfer {
    Receiving {
        id: u64,
        mime: String,
        total: usize,
        data: Vec<u8>,
    },
    /// Refused; the rest of its chunks are dropped.
    Refused(u64),
}

/// Assembles the chunks of each client's transfer and publishes the result.
pub struct Receiver {
    store: Store,
    limits: Limits,
    transfers: HashMap<u64, Transfer>,
}

impl Receiver {
    pub fn new(store: Store, limits: Limits) -> Self {
        Self {
            store,
            limits,
            transfers: HashMap::new(),
        }
    }

    pub fn max_bytes(&self) -> u64 {
        self.limits.max_bytes
    }

    pub fn drop_client(&mut self, client: u64) {
        self.transfers.remove(&client);
    }

    /// Take one chunk from `client`. Returns the ack to send, once the
    /// transfer is complete or refused.
    pub fn on_chunk(&mut self, client: u64, put: ClipboardPut) -> Option<ClipboardAck> {
        let id = put.transfer_id;
        let outcome = match self.transfers.remove(&client) {
            Some(Transfer::Refused(refused)) if refused == id => {
                self.transfers.insert(client, Transfer::Refused(id));
                return None;
            }
            Some(Transfer::Receiving {
                id: current,
                mime,
                total,
                data,
            }) if current == id => self.append(mime, total, data, &put),
            // A new transfer; whatever the client was sending before is void.
            _ => {
                self.store.clear();
                self.begin(&put)
                    .and_then(|(mime, total)| self.append(mime, total, Vec::new(), &put))
            }
        };
        match outcome {
            Ok(Some((mime, total, data))) => {
                self.transfers.insert(
                    client,
                    Transfer::Receiving {
                        id,
                        mime,
                        total,
                        data,
                    },
                );
                None
            }
            Ok(None) => Some(ClipboardAck {
                transfer_id: id,
                ok: true,
                error: String::new(),
            }),
            Err(error) => {
                if !put.last {
                    self.transfers.insert(client, Transfer::Refused(id));
                }
                Some(ClipboardAck {
                    transfer_id: id,
                    ok: false,
                    error,
                })
            }
        }
    }

    /// Check the first chunk of a transfer.
    fn begin(&self, put: &ClipboardPut) -> Result<(String, usize), String> {
        let mime = put.mime.trim();
        let valid = !mime.is_empty()
            && mime.len() <= MAX_MIME_LEN
            && mime.chars().all(|c| c.is_ascii_graphic());
        if !valid {
            return Err("clipboard content has no valid MIME type".into());
        }
        if put.total_size == 0 {
            return Err("clipboard content is empty".into());
        }
        if put.total_size > self.limits.max_bytes {
            return Err(format!(
                "clipboard content is too large: {}, the server accepts at most {}",
                human_size(put.total_size),
                human_size(self.limits.max_bytes),
            ));
        }
        Ok((mime.to_owned(), put.total_size as usize))
    }

    /// Add a chunk. `Ok(Some(..))` is the transfer still in progress,
    /// `Ok(None)` means it was the last chunk and the content is published.
    fn append(
        &self,
        mime: String,
        total: usize,
        mut data: Vec<u8>,
        put: &ClipboardPut,
    ) -> Result<Option<(String, usize, Vec<u8>)>, String> {
        if data.len() + put.data.len() > total {
            return Err("clipboard transfer carries more data than it announced".into());
        }
        data.extend_from_slice(&put.data);
        if !put.last {
            return Ok(Some((mime, total, data)));
        }
        if data.len() != total {
            return Err(format!(
                "clipboard transfer ended after {} of {total} bytes",
                data.len()
            ));
        }
        self.store
            .publish(&mime, &data, self.limits.ttl)
            .map_err(|err| format!("store clipboard content: {err}"))?;
        Ok(None)
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 || value.fract() < 0.05 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Parse a size like `20MB`, `512K` or `1048576`. Units are powers of 1024.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let (number, unit) = text.split_at(digits);
    let number: u64 = number.parse().map_err(|_| format!("not a size: {text}"))?;
    let shift = match unit.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 0,
        "K" | "KB" => 10,
        "M" | "MB" => 20,
        "G" | "GB" => 30,
        _ => return Err(format!("unknown size unit in {text}; use K, M or G")),
    };
    number
        .checked_mul(1 << shift)
        .ok_or_else(|| format!("size out of range: {text}"))
}

// ---------------------------------------------------------------------------
// xclip / wl-paste
// ---------------------------------------------------------------------------

/// What a clipboard tool was asked for.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    /// List the MIME types on offer.
    Types,
    /// Print the content as this type; `None` asks for text.
    Get(Option<String>),
    /// Anything the server's clipboard does not do: copying, other
    /// selections, watching. Left to the real tool.
    Other,
}

fn is_text_target(target: &str) -> bool {
    let target = target.to_ascii_lowercase();
    matches!(
        target.as_str(),
        "string" | "utf8_string" | "text" | "compound_text"
    ) || target.starts_with("text/")
}

fn get(target: Option<&str>) -> Request {
    match target {
        Some(target) if target.eq_ignore_ascii_case("TARGETS") => Request::Types,
        Some(target) if !is_text_target(target) => Request::Get(Some(target.to_owned())),
        _ => Request::Get(None),
    }
}

/// xclip takes any unambiguous prefix of an option: `-sel`, `-o`, `-t`.
fn parse_xclip(args: &[String]) -> Request {
    let mut out = false;
    let mut clipboard = false;
    let mut target = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let Some(option) = arg.strip_prefix('-').filter(|option| !option.is_empty()) else {
            // A file to copy from.
            return Request::Other;
        };
        if "selection".starts_with(option) && option.len() >= 3 {
            // Only the first letter of the selection name counts.
            clipboard = args.next().is_some_and(|name| name.starts_with(['c', 'C']));
        } else if "target".starts_with(option) {
            target = args.next().map(String::as_str);
        } else if "out".starts_with(option) {
            out = true;
        } else if "in".starts_with(option) {
            return Request::Other;
        } else if "display".starts_with(option) || "loops".starts_with(option) {
            args.next();
        }
    }
    if !out || !clipboard {
        return Request::Other;
    }
    get(target)
}

fn parse_wl_paste(args: &[String]) -> Request {
    let mut list = false;
    let mut target = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-l" | "--list-types" => list = true,
            "-t" | "--type" => target = args.next().map(String::as_str),
            "-s" | "--seat" => {
                args.next();
            }
            "-p" | "--primary" | "-w" | "--watch" => return Request::Other,
            _ => {
                if let Some(value) = arg.strip_prefix("--type=") {
                    target = Some(value);
                } else if arg.starts_with("--seat=") {
                } else if !arg.starts_with('-') {
                    return Request::Other;
                }
            }
        }
    }
    if list { Request::Types } else { get(target) }
}

fn same_mime(a: &str, b: &str) -> bool {
    let essence = |mime: &str| {
        mime.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    };
    essence(a) == essence(b)
}

/// The bytes to print for `request`, or why there are none.
fn answer(tool: &str, request: &Request, content: &Content) -> Result<Vec<u8>, String> {
    let text = is_text_target(&content.mime);
    match request {
        Request::Types => {
            let mut types = String::new();
            if tool == "xclip" {
                types.push_str("TARGETS\n");
            }
            types.push_str(&content.mime);
            types.push('\n');
            if text && !same_mime(&content.mime, "text/plain") {
                types.push_str("text/plain\n");
            }
            Ok(types.into_bytes())
        }
        Request::Get(None) if text => Ok(content.data.clone()),
        Request::Get(Some(mime)) if same_mime(mime, &content.mime) => Ok(content.data.clone()),
        Request::Get(wanted) => Err(format!(
            "target {} not available, the clipboard holds {}",
            wanted.as_deref().unwrap_or("text"),
            content.mime
        )),
        Request::Other => unreachable!("handled by the real tool"),
    }
}

/// Run as `xclip` or `wl-paste`: answer from the server's clipboard, or hand
/// over to the real tool when the clipboard is empty or the request is not
/// one it serves.
pub fn shim(tool: &str, exe: &Path, args: Vec<OsString>) -> ExitCode {
    let text_args: Option<Vec<String>> = args
        .iter()
        .map(|arg| arg.to_str().map(str::to_owned))
        .collect();
    let request = match (&text_args, tool) {
        (Some(args), "xclip") => parse_xclip(args),
        (Some(args), _) => parse_wl_paste(args),
        (None, _) => Request::Other,
    };
    let dir = std::env::var_os(DIR_ENV)
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| clipboard_dir(&runtime_dir()), PathBuf::from);
    let content = match request {
        Request::Other => None,
        _ => Store::at(dir).read(),
    };
    let Some(content) = content else {
        let missing = format!("the tilr clipboard is empty and no other {tool} is installed");
        return exec_next(tool, exe, &args, &missing);
    };
    match answer(tool, &request, &content) {
        Ok(bytes) => {
            let mut stdout = io::stdout().lock();
            match stdout.write_all(&bytes).and_then(|()| stdout.flush()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            }
        }
        Err(message) => {
            eprintln!("{tool}: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Replace this process with the next `tool` on PATH that is not this
/// executable; without one, say `missing`.
pub fn exec_next(tool: &str, exe: &Path, args: &[OsString], missing: &str) -> ExitCode {
    let own = fs::canonicalize(exe).ok();
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(tool);
        let Ok(resolved) = fs::canonicalize(&candidate) else {
            continue;
        };
        if Some(&resolved) == own.as_ref() || !is_executable(&resolved) {
            continue;
        }
        let err = std::process::Command::new(&candidate).args(args).exec();
        eprintln!("{tool}: run {}: {err}", candidate.display());
        return ExitCode::FAILURE;
    }
    eprintln!("{tool}: {missing}");
    ExitCode::FAILURE
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The shim to run as, if this process was started under one of their names.
pub fn shim_name(argv0: &OsStr) -> Option<&'static str> {
    let name = Path::new(argv0).file_name()?;
    SHIMS.into_iter().find(|shim| name == OsStr::new(shim))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("tilr-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::create(dir).unwrap()
    }

    fn receiver(name: &str, max_bytes: u64) -> Receiver {
        Receiver::new(
            temp_store(name),
            Limits {
                max_bytes,
                ttl: None,
            },
        )
    }

    fn chunk(id: u64, mime: &str, total: u64, data: &[u8], last: bool) -> ClipboardPut {
        ClipboardPut {
            transfer_id: id,
            mime: mime.into(),
            total_size: total,
            data: bytes::Bytes::copy_from_slice(data),
            last,
        }
    }

    #[test]
    fn xclip_requests() {
        // The commands Claude Code runs.
        assert_eq!(
            parse_xclip(&args("-selection clipboard -t TARGETS -o")),
            Request::Types
        );
        assert_eq!(
            parse_xclip(&args("-selection clipboard -t image/png -o")),
            Request::Get(Some("image/png".into()))
        );
        assert_eq!(
            parse_xclip(&args("-selection clipboard -t text/plain -o")),
            Request::Get(None)
        );
        // Abbreviations and the default target.
        assert_eq!(parse_xclip(&args("-sel c -o")), Request::Get(None));
        assert_eq!(
            parse_xclip(&args("-o -sel clip -target UTF8_STRING")),
            Request::Get(None)
        );
        // Not ours: copying, the primary selection, a file argument.
        assert_eq!(parse_xclip(&args("-selection clipboard")), Request::Other);
        assert_eq!(
            parse_xclip(&args("-selection clipboard -i")),
            Request::Other
        );
        assert_eq!(parse_xclip(&args("-o")), Request::Other);
        assert_eq!(parse_xclip(&args("-selection primary -o")), Request::Other);
        assert_eq!(
            parse_xclip(&args("-selection clipboard -o file.txt")),
            Request::Other
        );
    }

    #[test]
    fn wl_paste_requests() {
        assert_eq!(parse_wl_paste(&args("-l")), Request::Types);
        assert_eq!(parse_wl_paste(&args("--list-types")), Request::Types);
        assert_eq!(
            parse_wl_paste(&args("--type image/png")),
            Request::Get(Some("image/png".into()))
        );
        assert_eq!(
            parse_wl_paste(&args("--type=image/bmp")),
            Request::Get(Some("image/bmp".into()))
        );
        assert_eq!(parse_wl_paste(&args("-t text/plain")), Request::Get(None));
        assert_eq!(parse_wl_paste(&args("")), Request::Get(None));
        assert_eq!(parse_wl_paste(&args("--no-newline")), Request::Get(None));
        assert_eq!(parse_wl_paste(&args("--primary")), Request::Other);
        assert_eq!(parse_wl_paste(&args("--watch cat")), Request::Other);
    }

    #[test]
    fn answers_for_an_image() {
        let image = Content {
            mime: "image/png".into(),
            data: vec![1, 2, 3],
        };
        assert_eq!(
            answer("xclip", &Request::Types, &image).unwrap(),
            b"TARGETS\nimage/png\n"
        );
        assert_eq!(
            answer("wl-paste", &Request::Types, &image).unwrap(),
            b"image/png\n"
        );
        assert_eq!(
            answer("xclip", &Request::Get(Some("image/png".into())), &image).unwrap(),
            [1, 2, 3]
        );
        // Asking for text or another format must fail, not print the image.
        assert!(answer("xclip", &Request::Get(None), &image).is_err());
        assert!(answer("xclip", &Request::Get(Some("image/bmp".into())), &image).is_err());
    }

    #[test]
    fn answers_for_text() {
        let text = Content {
            mime: "text/plain;charset=utf-8".into(),
            data: b"hello".to_vec(),
        };
        assert_eq!(
            answer("xclip", &Request::Get(None), &text).unwrap(),
            b"hello"
        );
        assert!(answer("xclip", &Request::Get(Some("image/png".into())), &text).is_err());
    }

    #[test]
    fn store_round_trip_and_expiry() {
        let store = temp_store("store");
        assert_eq!(store.read(), None);
        // Content may hold newlines and bytes that are not text.
        let data = b"\x89PNG\n\n\x00\xff".to_vec();
        store.publish("image/png", &data, None).unwrap();
        assert_eq!(
            store.read(),
            Some(Content {
                mime: "image/png".into(),
                data
            })
        );
        store.clear();
        assert_eq!(store.read(), None);

        store
            .publish("text/plain", b"old", Some(Duration::from_secs(60)))
            .unwrap();
        assert!(store.read().is_some());
        // Rewrite the expiry into the past.
        fs::write(store.file(), b"text/plain\n1\nold").unwrap();
        assert_eq!(store.read(), None);
    }

    #[test]
    fn transfer_in_chunks_publishes_on_the_last() {
        let mut receiver = receiver("chunks", 100);
        assert_eq!(
            receiver.on_chunk(1, chunk(7, "image/png", 5, b"ab", false)),
            None
        );
        assert_eq!(receiver.store.read(), None);
        assert_eq!(receiver.on_chunk(1, chunk(7, "", 0, b"cd", false)), None);
        let ack = receiver.on_chunk(1, chunk(7, "", 0, b"e", true)).unwrap();
        assert!(ack.ok, "{}", ack.error);
        assert_eq!(ack.transfer_id, 7);
        assert_eq!(receiver.store.read().unwrap().data, b"abcde");
    }

    #[test]
    fn oversized_transfer_is_refused_and_leaves_the_clipboard_empty() {
        let mut receiver = receiver("oversized", 4);
        receiver.store.publish("image/png", b"old", None).unwrap();
        let ack = receiver
            .on_chunk(1, chunk(1, "image/png", 1 << 30, b"ab", false))
            .unwrap();
        assert!(!ack.ok);
        assert!(ack.error.contains("too large: 1 GB"), "{}", ack.error);
        assert!(ack.error.contains("at most 4 B"), "{}", ack.error);
        // The rest of the refused transfer is dropped without another ack.
        assert_eq!(receiver.on_chunk(1, chunk(1, "", 0, b"cd", true)), None);
        assert_eq!(receiver.store.read(), None);
        // The client can start over.
        let ack = receiver
            .on_chunk(1, chunk(2, "image/png", 2, b"ok", true))
            .unwrap();
        assert!(ack.ok);
    }

    #[test]
    fn transfer_that_lies_about_its_size_is_refused() {
        let mut receiver = receiver("lying", 100);
        assert_eq!(
            receiver.on_chunk(1, chunk(1, "image/png", 3, b"ab", false)),
            None
        );
        let ack = receiver
            .on_chunk(1, chunk(1, "", 0, b"cdef", false))
            .unwrap();
        assert!(!ack.ok);
        assert_eq!(receiver.store.read(), None);

        let ack = receiver
            .on_chunk(1, chunk(2, "image/png", 9, b"ab", true))
            .unwrap();
        assert!(!ack.ok);
        assert!(ack.error.contains("2 of 9"), "{}", ack.error);
        assert_eq!(receiver.store.read(), None);
    }

    #[test]
    fn transfer_without_a_type_is_refused() {
        let mut receiver = receiver("untyped", 100);
        assert!(
            !receiver
                .on_chunk(1, chunk(1, "", 2, b"ab", true))
                .unwrap()
                .ok
        );
        assert!(
            !receiver
                .on_chunk(1, chunk(2, "image/png\nx", 2, b"ab", true))
                .unwrap()
                .ok
        );
        assert!(
            !receiver
                .on_chunk(1, chunk(3, "image/png", 0, b"", true))
                .unwrap()
                .ok
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("20MB"), Ok(20 << 20));
        assert_eq!(parse_size("512k"), Ok(512 << 10));
        assert_eq!(parse_size("1G"), Ok(1 << 30));
        assert_eq!(parse_size("4096"), Ok(4096));
        assert!(parse_size("lots").is_err());
        assert!(parse_size("5 parsecs").is_err());
        assert_eq!(human_size(900), "900 B");
        assert_eq!(human_size(20 << 20), "20 MB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(1 << 30), "1 GB");
    }
}
