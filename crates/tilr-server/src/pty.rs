//! Pty creation and the few raw syscalls around it.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};

pub struct Spawned {
    pub master: OwnedFd,
    pub child: Child,
}

/// A child process: started by this process, or inherited through a reload,
/// in which case it is a child all the same but tokio does not know it.
pub enum Process {
    Spawned(Child),
    Inherited(u32),
}

impl Process {
    pub fn pid(&self) -> Option<u32> {
        match self {
            Self::Spawned(child) => child.id(),
            Self::Inherited(pid) => Some(*pid),
        }
    }

    /// Wait for the process to exit. `None` if its exit code cannot be had.
    pub async fn wait(&mut self) -> Option<i32> {
        match self {
            Self::Spawned(child) => child.wait().await.ok()?.code(),
            Self::Inherited(pid) => {
                let pid = *pid;
                let code = tokio::task::spawn_blocking(move || wait_pid(pid)).await;
                code.ok()?.ok()
            }
        }
    }
}

/// Name of the variable that tells what runs in a pane which pane it is in.
pub const PANE_ENV: &str = "TILR_PANE";

/// Variables a launchd job hands its children that mean nothing to a shell.
const LAUNCHD_ENV: [&str; 2] = ["XPC_SERVICE_NAME", "XPC_FLAGS"];

/// The user this process runs as, from the password database.
#[derive(Debug, Default)]
struct User {
    name: Option<String>,
    home: Option<String>,
    shell: Option<String>,
}

impl User {
    fn current() -> Self {
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let mut buf = vec![0u8; 4096];
        loop {
            // SAFETY: an all-zero passwd is a valid out-value.
            let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
            let mut found: *mut libc::passwd = std::ptr::null_mut();
            // SAFETY: the out-pointers are valid and `buf` outlives the
            // strings `pwd` points into, which are copied before it goes.
            let rc = unsafe {
                libc::getpwuid_r(
                    uid,
                    &mut pwd,
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    &mut found,
                )
            };
            if rc == libc::ERANGE && buf.len() < 1 << 20 {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            if rc != 0 || found.is_null() {
                return Self::default();
            }
            let field = |ptr: *const libc::c_char| {
                if ptr.is_null() {
                    return None;
                }
                // SAFETY: a non-null passwd field is a NUL-terminated string in `buf`.
                let text = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy();
                (!text.is_empty()).then(|| text.into_owned())
            };
            return Self {
                name: field(pwd.pw_name),
                home: field(pwd.pw_dir),
                shell: field(pwd.pw_shell),
            };
        }
    }
}

/// The user's login shell: the password database's, else `$SHELL`, else
/// `/bin/sh`.
fn login_shell(user: &User) -> String {
    user.shell
        .clone()
        .or_else(|| {
            std::env::var("SHELL")
                .ok()
                .filter(|shell| !shell.is_empty())
        })
        .unwrap_or_else(|| "/bin/sh".into())
}

/// The name a login shell is started under: its own with a `-` in front,
/// which every Unix shell takes to mean "read the login profile".
fn login_arg0(shell: &str) -> String {
    let name = std::path::Path::new(shell)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| shell.to_owned());
    format!("-{name}")
}

/// Open a pty and start `command` on its slave side as a session leader;
/// with no command, the user's login shell as a login shell, so that it reads
/// the user's profile whatever environment the server itself was started in
/// (a launchd agent gets the bare system PATH).
pub fn spawn_shell(
    pane_id: &str,
    command: &[String],
    cwd: &str,
    cols: u16,
    rows: u16,
) -> Result<Spawned> {
    let (master, slave) = open_pty(cols, rows).context("openpty")?;

    let user = User::current();
    let shell = login_shell(&user);
    let mut cmd = match command.split_first() {
        Some((program, args)) => {
            let mut cmd = Command::new(program);
            cmd.args(args);
            cmd
        }
        None => {
            let mut cmd = Command::new(&shell);
            cmd.arg0(login_arg0(&shell));
            cmd
        }
    };
    let program = command.first().unwrap_or(&shell).clone();

    for (name, value) in [
        ("HOME", &user.home),
        ("USER", &user.name),
        ("LOGNAME", &user.name),
    ] {
        if std::env::var_os(name).is_none_or(|value| value.is_empty())
            && let Some(value) = value
        {
            cmd.env(name, value);
        }
    }
    for name in LAUNCHD_ENV {
        cmd.env_remove(name);
    }
    cmd.env("SHELL", &shell)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env(PANE_ENV, pane_id)
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    if !cwd.is_empty() {
        cmd.current_dir(cwd);
    }
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            // Make the pty (already on stdin) the controlling terminal.
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn().with_context(|| format!("spawn {program}"))?;
    Ok(Spawned { master, child })
}

fn open_pty(cols: u16, rows: u16) -> io::Result<(OwnedFd, OwnedFd)> {
    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    let mut size = winsize(cols, rows);
    // SAFETY: out-pointers are valid; name and termios are optional.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            // A raw pointer: Linux takes `*const winsize`, macOS `*mut`.
            &raw mut size,
        )
    };
    if rc == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty returned two fresh descriptors that we now own.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    // Neither end may leak into the shell; it gets the slave as its stdio.
    set_cloexec(master.as_raw_fd(), true)?;
    set_cloexec(slave.as_raw_fd(), true)?;
    Ok((master, slave))
}

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

pub fn set_window_size(fd: BorrowedFd<'_>, cols: u16, rows: u16) -> io::Result<()> {
    let size = winsize(cols, rows);
    // SAFETY: fd is a live pty master and `size` outlives the call.
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCSWINSZ, &size) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Set whether `fd` is closed when this process execs.
pub fn set_cloexec(fd: RawFd, close: bool) -> io::Result<()> {
    let flags = if close { libc::FD_CLOEXEC } else { 0 };
    // SAFETY: plain fcntl on a descriptor.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Block until the child `pid` exits and return its exit code, or -1 if a
/// signal ended it.
pub fn wait_pid(pid: u32) -> io::Result<i32> {
    let mut status = 0;
    loop {
        // SAFETY: `status` is a valid out-pointer.
        if unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) } != -1 {
            break;
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
    Ok(if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        -1
    })
}

pub fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: plain fcntl on a descriptor.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Signal the whole process group of a session leader.
pub fn signal_group(pid: u32, signal: libc::c_int) {
    // SAFETY: killpg has no memory-safety preconditions.
    unsafe {
        libc::killpg(pid as libc::pid_t, signal);
    }
}

pub fn read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: buf is valid for writes of buf.len() bytes.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn write(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: buf is valid for reads of buf.len() bytes.
    let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_shell_is_started_under_its_name_with_a_dash() {
        assert_eq!(login_arg0("/bin/zsh"), "-zsh");
        assert_eq!(login_arg0("/opt/homebrew/bin/fish"), "-fish");
        assert_eq!(login_arg0("sh"), "-sh");
    }

    #[test]
    fn the_login_shell_comes_from_the_password_database_first() {
        let user = User {
            shell: Some("/bin/zsh".into()),
            ..User::default()
        };
        assert_eq!(login_shell(&user), "/bin/zsh");
        // This process's user has an entry, with a home at least.
        assert!(User::current().home.is_some());
    }
}
