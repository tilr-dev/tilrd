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
    /// With a pidfd for it where there is one (Linux 5.3+), opened when it is
    /// taken over: tokio holds one for each child it spawns, so a process
    /// holds the same descriptors before and after a reload, and waiting for
    /// the child holds no thread.
    Inherited(u32, Option<Pidfd>),
}

#[cfg(target_os = "linux")]
pub type Pidfd = tokio::io::unix::AsyncFd<OwnedFd>;
#[cfg(not(target_os = "linux"))]
pub type Pidfd = std::convert::Infallible;

impl Process {
    /// A child this process got from the one it replaced. Call inside the
    /// runtime.
    pub fn inherited(pid: u32) -> Self {
        Self::Inherited(pid, open_pidfd(pid))
    }

    pub fn pid(&self) -> Option<u32> {
        match self {
            Self::Spawned(child) => child.id(),
            Self::Inherited(pid, _) => Some(*pid),
        }
    }

    /// Wait for the process to exit. `None` if its exit code cannot be had.
    pub async fn wait(&mut self) -> Option<i32> {
        match self {
            Self::Spawned(child) => child.wait().await.ok()?.code(),
            Self::Inherited(pid, pidfd) => {
                let pid = *pid;
                #[cfg(target_os = "linux")]
                if let Some(pidfd) = pidfd
                    && pidfd.readable().await.is_ok()
                {
                    // It has exited: the wait does not block.
                    return wait_pid(pid).ok();
                }
                #[cfg(not(target_os = "linux"))]
                let _ = pidfd;
                let code = tokio::task::spawn_blocking(move || wait_pid(pid)).await;
                code.ok()?.ok()
            }
        }
    }
}

/// A pidfd for child `pid`, watched by tokio's reactor; readable once the
/// child has exited. `None` where there is none to be had.
#[cfg(target_os = "linux")]
fn open_pidfd(pid: u32) -> Option<Pidfd> {
    use tokio::io::Interest;

    // SAFETY: pidfd_open takes a pid and flags; it returns a new descriptor,
    // close-on-exec, or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    let fd = RawFd::try_from(fd).ok().filter(|&fd| fd >= 0)?;
    // SAFETY: the descriptor was just made and is owned by nobody else.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: `fd` is an `OwnedFd` moved into the `AsyncFd`, so it stays open
    // and unchanged until the `AsyncFd` drops it, and nothing swaps it out.
    unsafe { Pidfd::register_with_interest(fd, Interest::READABLE) }.ok()
}

#[cfg(not(target_os = "linux"))]
fn open_pidfd(_pid: u32) -> Option<Pidfd> {
    None
}

/// Name of the variable that tells what runs in a pane which pane it is in.
pub const PANE_ENV: &str = "TILR_PANE";

/// Variables a launchd job hands its children that mean nothing to a shell.
const LAUNCHD_ENV: [&str; 2] = ["XPC_SERVICE_NAME", "XPC_FLAGS"];

/// Variables systemd hands the server (a `Type=notify` unit, socket
/// activation, the watchdog) that are about the server only: a program in a
/// pane or an exec that knows them would act on them for a unit it is not.
pub const SERVICE_MANAGER_ENV: [&str; 6] = [
    "NOTIFY_SOCKET",
    "LISTEN_FDS",
    "LISTEN_FDNAMES",
    "LISTEN_PID",
    "WATCHDOG_USEC",
    "WATCHDOG_PID",
];

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

/// How a login shell is started: the name it runs under, its arguments and
/// what is added to its environment.
#[derive(Debug, PartialEq, Eq)]
pub struct LoginPlan {
    pub arg0: String,
    pub args: Vec<String>,
    pub env: Vec<(String, std::ffi::OsString)>,
}

/// How to start the login shell `shell`: as `-<name>`, and for zsh, bash
/// and fish through the server's startup wrapper (see `shell`) when the
/// panes have tools to keep first on PATH (`shims`) and the wrapper is in
/// `wrappers`. `env` reads the environment the shell would get.
pub fn login_plan(
    shell: &str,
    wrappers: Option<&std::path::Path>,
    shims: bool,
    env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
) -> LoginPlan {
    use crate::shell;
    let arg0 = login_arg0(shell);
    let mut plan = LoginPlan {
        arg0,
        args: Vec::new(),
        env: Vec::new(),
    };
    if !shims {
        return plan;
    }
    let wrapped = |name| wrappers.filter(|dir| shell::has_wrapper(dir, name));
    match plan.arg0.trim_start_matches('-') {
        "zsh" => {
            if let Some(dir) = wrapped("zsh") {
                if let Some(own) = env("ZDOTDIR") {
                    plan.env.push((shell::ZDOTDIR_ENV.into(), own));
                }
                plan.env
                    .push(("ZDOTDIR".into(), dir.join("zsh").into_os_string()));
            }
        }
        "bash" => {
            if let Some(dir) = wrapped("bash") {
                plan.args.push("--posix".into());
                plan.env
                    .push(("ENV".into(), dir.join("bash/env.sh").into_os_string()));
                // In posix mode bash's history would be ~/.sh_history.
                if env("HISTFILE").is_none()
                    && let Some(home) = env("HOME")
                {
                    let mut file = std::path::PathBuf::from(home);
                    file.push(".bash_history");
                    plan.env.push(("HISTFILE".into(), file.into_os_string()));
                    plan.env.push((shell::HISTFILE_ENV.into(), "1".into()));
                }
            }
        }
        "fish" => {
            plan.args.extend([
                "--login".into(),
                "--init-command".into(),
                shell::FISH_INIT.into(),
            ]);
        }
        _ => {}
    }
    plan
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
            let shims =
                std::env::var_os(crate::clipboard::SHIMS_ENV).is_some_and(|dir| !dir.is_empty());
            let wrappers = crate::shell::wrapper_dir(&crate::clipboard::runtime_dir());
            let home = user.home.clone();
            let env = |name: &str| {
                std::env::var_os(name)
                    .filter(|value| !value.is_empty())
                    .or_else(|| {
                        (name == "HOME")
                            .then(|| home.clone().map(Into::into))
                            .flatten()
                    })
            };
            let plan = login_plan(&shell, Some(&wrappers), shims, &env);
            let mut cmd = Command::new(&shell);
            cmd.arg0(&plan.arg0).args(&plan.args);
            for (name, value) in plan.env {
                cmd.env(name, value);
            }
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
    for name in LAUNCHD_ENV.into_iter().chain(SERVICE_MANAGER_ENV) {
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

/// The foreground process group of the terminal whose master is `fd`, if
/// it has one.
pub fn foreground_group(fd: BorrowedFd<'_>) -> Option<libc::pid_t> {
    // SAFETY: tcgetpgrp has no memory-safety preconditions.
    let pgid = unsafe { libc::tcgetpgrp(fd.as_raw_fd()) };
    (pgid > 0).then_some(pgid)
}

/// Whether any process is left in the process group `pgid`; a zombie not
/// yet reaped counts.
pub fn group_alive(pgid: libc::pid_t) -> bool {
    // SAFETY: kill with signal 0 only checks.
    let alive = unsafe { libc::kill(-pgid, 0) } == 0;
    alive || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
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
    fn a_login_shell_is_wrapped_by_its_name() {
        let runtime = std::env::temp_dir().join(format!("tilrd-pty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let dir = crate::shell::install_wrappers(&runtime).unwrap();
        let env = |name: &str| match name {
            "HOME" => Some("/home/u".into()),
            "ZDOTDIR" => Some("/home/u/.config/zsh".into()),
            _ => None,
        };
        let os = |text: &str| std::ffi::OsString::from(text);

        let zsh = login_plan("/bin/zsh", Some(&dir), true, &env);
        assert_eq!(zsh.arg0, "-zsh");
        assert!(zsh.args.is_empty());
        assert_eq!(
            zsh.env,
            [
                ("TILR_ZDOTDIR".into(), os("/home/u/.config/zsh")),
                ("ZDOTDIR".into(), dir.join("zsh").into_os_string()),
            ]
        );
        // No ZDOTDIR of the user's: none to keep.
        let bare = login_plan("/bin/zsh", Some(&dir), true, &|_| None);
        assert_eq!(
            bare.env,
            [("ZDOTDIR".into(), dir.join("zsh").into_os_string())]
        );

        let bash = login_plan("/usr/local/bin/bash", Some(&dir), true, &env);
        assert_eq!(bash.arg0, "-bash");
        assert_eq!(bash.args, ["--posix"]);
        assert_eq!(
            bash.env,
            [
                ("ENV".into(), dir.join("bash/env.sh").into_os_string()),
                ("HISTFILE".into(), os("/home/u/.bash_history")),
                ("TILR_BASH_UNEXPORT_HISTFILE".into(), os("1")),
            ]
        );

        let fish = login_plan("/usr/bin/fish", Some(&dir), true, &env);
        assert_eq!(fish.arg0, "-fish");
        assert_eq!(
            fish.args,
            ["--login", "--init-command", crate::shell::FISH_INIT]
        );
        assert!(fish.env.is_empty());

        // Any other shell, or no tools for the panes: as before.
        for (shell, shims) in [("/bin/sh", true), ("/bin/dash", true), ("/bin/zsh", false)] {
            let plan = login_plan(shell, Some(&dir), shims, &env);
            assert_eq!(plan.arg0, login_arg0(shell));
            assert!(plan.args.is_empty() && plan.env.is_empty(), "{shell}");
        }
        std::fs::remove_dir_all(&runtime).unwrap();
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
