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

/// Open a pty and start `command` (or the login shell) on its slave side as a
/// session leader.
pub fn spawn_shell(command: &[String], cwd: &str, cols: u16, rows: u16) -> Result<Spawned> {
    let (master, slave) = open_pty(cols, rows).context("openpty")?;

    let shell;
    let (program, args) = match command.split_first() {
        Some((program, args)) => (program.as_str(), args),
        None => {
            shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            (shell.as_str(), &[][..])
        }
    };

    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
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
            &mut size,
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
