//! Test-only ownership of real servers, independent of their command sockets.
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

struct Server {
    stream: UnixStream,
    pid: u32,
    watcher: Option<OwnedFd>,
}

pub struct Owner {
    path: PathBuf,
    stop: UnixStream,
    worker: Option<std::thread::JoinHandle<Vec<Server>>>,
}

impl Owner {
    pub fn new(dir: &Path) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!(
            "owner-{}.sock",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (stop, stop_read) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            let mut servers = Vec::new();
            while accept_ready(&listener, &stop_read) {
                let (mut stream, _) = listener.accept().unwrap();
                let mut pid = [0; 4];
                if stream.read_exact(&mut pid).is_err() {
                    continue;
                }
                let pid = u32::from_ne_bytes(pid);
                let watcher = watch(pid);
                // Register the exit event before allowing server startup.
                if stream.write_all(b"r").is_ok() {
                    servers.push(Server {
                        stream,
                        pid,
                        watcher,
                    });
                }
            }
            servers
        });
        Self {
            path,
            stop,
            worker: Some(worker),
        }
    }

    pub fn configure(&self, command: &mut Command) {
        command.env("TILR_TEST_OWNER_SOCKET", &self.path);
    }

    pub fn close(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let _ = self.stop.write_all(b"s");
        let servers = worker.join().expect("server owner thread");
        let _ = std::fs::remove_file(&self.path);
        for server in &servers {
            let _ = server.stream.shutdown(std::net::Shutdown::Both);
        }
        for Server { pid, watcher, .. } in servers {
            if let Some(watcher) = watcher {
                let exited = wait(&watcher);
                if !exited {
                    // Only an exact PID registered while it was alive is owned.
                    // SAFETY: kill has no memory-safety preconditions.
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                    let _ = wait(&watcher);
                    assert!(
                        std::thread::panicking(),
                        "owned server {pid} did not shut down"
                    );
                    eprintln!("owned server {pid} needed SIGKILL during unwinding");
                }
            }
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.close();
    }
}

pub fn watch(pid: u32) -> Option<OwnedFd> {
    // SAFETY: all calls use valid scalar arguments and initialized event storage.
    let fd = unsafe {
        #[cfg(target_os = "macos")]
        {
            let fd = libc::kqueue();
            assert!(fd >= 0, "kqueue: {}", std::io::Error::last_os_error());
            let event = libc::kevent {
                ident: pid as usize,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ENABLE,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            if libc::kevent(fd, &event, 1, std::ptr::null_mut(), 0, std::ptr::null()) < 0 {
                let error = std::io::Error::last_os_error();
                libc::close(fd);
                if error.raw_os_error() == Some(libc::ESRCH) {
                    return None;
                }
                panic!("watch server {pid}: {error}");
            }
            fd
        }
        #[cfg(target_os = "linux")]
        {
            let fd = libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32;
            if fd < 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    return None;
                }
                panic!("watch server {pid}: {error}");
            }
            fd
        }
    };
    // SAFETY: fd was freshly opened above.
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub fn wait(watcher: &OwnedFd) -> bool {
    let mut event = libc::pollfd {
        fd: watcher.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // Twenty seconds bounds failure; the process exit event orders success.
        // SAFETY: event is valid storage for one pollfd.
        let result = unsafe { libc::poll(&mut event, 1, 20_000) };
        if result >= 0 {
            return result > 0 && event.revents & libc::POLLIN != 0;
        }
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::Interrupted
        );
    }
}

// A descriptor wakes the worker even if the test removed its scratch directory.
fn accept_ready(listener: &UnixListener, stop: &UnixStream) -> bool {
    let mut events = [
        libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: stop.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    loop {
        // SAFETY: events has storage for both pollfd entries.
        if unsafe { libc::poll(events.as_mut_ptr(), 2, -1) } >= 0 {
            return events[1].revents == 0;
        }
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::Interrupted
        );
    }
}
